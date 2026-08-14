//! Runtime-owned live implementations and affine execution owners.
//!
//! Runtime is the only layer allowed to turn a durable preparation into a live call.  The
//! transition is consuming and checks the Store's direct-new disposition before any provider
//! future can be created.  This module has no scheduler, per-run execution lock, or history API.

use std::any::{Any, TypeId};
use std::future::Future;
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use mfm_capabilities::{AccessCapabilityContract, ProposedStateOutcome};
use mfm_ids::{
    short_stable_id_fragment, ContentRef, RunId, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::single_trust::{PreparationRef, SequentialControlAddress};
use mfm_program::{
    canonical_value, state_implementation_ref, BindingDescriptor, Program, ProgramCatalog,
    ProgramDocument, QualifiedTypedValue, State,
};
use mfm_store::single_trust::{AppendDisposition, FactContinuation, SelectedRun};

/// Send-owned future used by one qualified live implementation.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Result type for Runtime assembly and affine execution transitions.
pub type Result<T> = std::result::Result<T, RuntimeError>;

/// Redaction-safe Runtime error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeError {
    /// The supplied catalog, contract, or binding identity is not exact.
    #[error("runtime identity is invalid")]
    Identity,
    /// The implementation or adapter is registered under the wrong mode.
    #[error("runtime execution mode is invalid")]
    Mode,
    /// Preparation did not produce a direct-new Store owner.
    #[error("runtime preparation was not committed as direct-new")]
    PreparationNotCommitted,
    /// A bounded preparation callback rejected the input.
    #[error("state preparation failed")]
    Preparation,
    /// The Store rejected a conclusion candidate.
    #[error("runtime conclusion could not be prepared")]
    Conclusion,
    /// A typed value or callback result could not be qualified.
    #[error("runtime typed value is invalid")]
    Value,
    /// The provider result is not trustworthy enough to conclude the occurrence.
    #[error("runtime access result is unresolved")]
    Unresolved,
    /// The Runtime's bounded work envelope is currently exhausted or invalid.
    #[error("runtime work capacity is unavailable")]
    Capacity,
}

impl From<mfm_program::single_trust::ProgramError> for RuntimeError {
    fn from(_: mfm_program::single_trust::ProgramError) -> Self {
        Self::Value
    }
}

impl From<mfm_store::single_trust::StoreError> for RuntimeError {
    fn from(_: mfm_store::single_trust::StoreError) -> Self {
        Self::Conclusion
    }
}

/// Bounded error returned before a provider-entering call exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("state preparation rejected")]
pub struct PreparationError;

/// Stable classification retained by a process-local unresolved access owner.
///
/// The classification is operational evidence only.  It is never persisted and it grants no
/// retry authority; the Store reducer remains the sole source of any later replacement action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnresolvedClassification {
    /// The response was malformed or could not be authenticated.
    InvalidResponse,
    /// Transport acknowledgement did not prove whether provider entry occurred.
    AcknowledgementUnknown,
    /// The owner was cancelled before a trustworthy handoff.
    Cancelled,
}

type AcceptedOutcomeParts<S, C> = (
    Arc<RuntimeAssemblyBrand>,
    QualifiedTypedValue<<S as State>::Input>,
    StableId,
    <C as AccessCapabilityContract>::Intent,
    <C as AccessCapabilityContract>::Evidence,
    PreparationRef,
    Option<FactContinuation>,
);

/// Affine capability evidence accepted by exactly one consuming committed call.
pub struct AcceptedOutcomeAccess<S: State, C: AccessCapabilityContract> {
    assembly_brand: Arc<RuntimeAssemblyBrand>,
    input: QualifiedTypedValue<S::Input>,
    intent: C::Intent,
    evidence: C::Evidence,
    call_id: StableId,
    preparation: PreparationRef,
    fact_continuation: Option<FactContinuation>,
}

impl<S: State, C: AccessCapabilityContract> AcceptedOutcomeAccess<S, C> {
    fn new(
        assembly_brand: Arc<RuntimeAssemblyBrand>,
        input: QualifiedTypedValue<S::Input>,
        intent: C::Intent,
        evidence: C::Evidence,
        call_id: StableId,
        preparation: PreparationRef,
        fact_continuation: Option<FactContinuation>,
    ) -> Result<Self> {
        C::bind_evidence(&intent, &evidence).map_err(|_| RuntimeError::Value)?;
        Ok(Self {
            assembly_brand,
            input,
            intent,
            evidence,
            call_id,
            preparation,
            fact_continuation,
        })
    }

    /// Returns accepted evidence to the owning State implementation.
    pub fn evidence(&self) -> &C::Evidence {
        &self.evidence
    }

    /// Borrows the exact cumulative input retained through adapter ingress.
    pub const fn input(&self) -> &QualifiedTypedValue<S::Input> {
        &self.input
    }

    /// Returns the exact call correlation.
    pub const fn call_id(&self) -> &StableId {
        &self.call_id
    }

    pub(crate) fn into_parts(self) -> AcceptedOutcomeParts<S, C> {
        (
            self.assembly_brand,
            self.input,
            self.call_id,
            self.intent,
            self.evidence,
            self.preparation,
            self.fact_continuation,
        )
    }
}

/// Affine evidence for a capability-specific integrity block.
///
/// The wrapper has no success constructor and therefore cannot accidentally grant a successor
/// context or retry authority. Its consuming failure route is static and does not invoke State
/// interpretation.
pub struct AcceptedIntegrityAccess<S: State, C: AccessCapabilityContract> {
    assembly_brand: Arc<RuntimeAssemblyBrand>,
    input: QualifiedTypedValue<S::Input>,
    intent: C::Intent,
    evidence: C::Evidence,
    call_id: StableId,
    preparation: PreparationRef,
    fact_continuation: Option<FactContinuation>,
}

impl<S: State, C: AccessCapabilityContract> AcceptedIntegrityAccess<S, C> {
    fn new(
        assembly_brand: Arc<RuntimeAssemblyBrand>,
        input: QualifiedTypedValue<S::Input>,
        intent: C::Intent,
        evidence: C::Evidence,
        call_id: StableId,
        preparation: PreparationRef,
        fact_continuation: Option<FactContinuation>,
    ) -> Result<Self> {
        C::bind_evidence(&intent, &evidence).map_err(|_| RuntimeError::Value)?;
        Ok(Self {
            assembly_brand,
            input,
            intent,
            evidence,
            call_id,
            preparation,
            fact_continuation,
        })
    }

    /// Returns the accepted integrity evidence for State-owned diagnostics or mapping.
    pub fn evidence(&self) -> &C::Evidence {
        &self.evidence
    }

    pub(crate) fn into_parts(self) -> AcceptedOutcomeParts<S, C> {
        (
            self.assembly_brand,
            self.input,
            self.call_id,
            self.intent,
            self.evidence,
            self.preparation,
            self.fact_continuation,
        )
    }
}

type UnresolvedParts<S, C> = (
    Arc<RuntimeAssemblyBrand>,
    QualifiedTypedValue<<S as State>::Input>,
    StableId,
    <C as AccessCapabilityContract>::Intent,
    PreparationRef,
    Option<FactContinuation>,
    UnresolvedClassification,
);

/// An accepted access result that cannot be interpreted into a durable conclusion.
pub struct UnresolvedAccess<S: State, C: AccessCapabilityContract> {
    assembly_brand: Arc<RuntimeAssemblyBrand>,
    input: QualifiedTypedValue<S::Input>,
    intent: C::Intent,
    call_id: StableId,
    preparation: PreparationRef,
    fact_continuation: Option<FactContinuation>,
    classification: UnresolvedClassification,
}

impl<S: State, C: AccessCapabilityContract> UnresolvedAccess<S, C> {
    fn new(
        assembly_brand: Arc<RuntimeAssemblyBrand>,
        input: QualifiedTypedValue<S::Input>,
        intent: C::Intent,
        call_id: StableId,
        preparation: PreparationRef,
        fact_continuation: Option<FactContinuation>,
        classification: UnresolvedClassification,
    ) -> Self {
        Self {
            assembly_brand,
            input,
            intent,
            call_id,
            preparation,
            fact_continuation,
            classification,
        }
    }

    /// Returns the stable process-local classification without exposing provider details.
    pub const fn classification(&self) -> UnresolvedClassification {
        self.classification
    }

    pub(crate) fn into_parts(self) -> UnresolvedParts<S, C> {
        (
            self.assembly_brand,
            self.input,
            self.call_id,
            self.intent,
            self.preparation,
            self.fact_continuation,
            self.classification,
        )
    }
}

/// Strict adapter result algebra.  Only bound adapter ingress can supply its payloads.
pub enum AccessResolution<S: State, C: AccessCapabilityContract> {
    /// Evidence is safe for the State's typed interpretation path.
    Outcome(AcceptedOutcomeAccess<S, C>),
    /// Evidence is an integrity block and may only become the declared failure route.
    BlockedIntegrity(AcceptedIntegrityAccess<S, C>),
    /// No trustworthy conclusion exists; the preparation remains neutral.
    Unresolved(UnresolvedAccess<S, C>),
}

type PureEvaluator<S> = dyn Fn(<S as State>::Input) -> ProposedStateOutcome<<S as State>::Output, <S as State>::Failure>
    + Send
    + Sync;
type AccessPreparer<S, C> = dyn Fn(
        &<S as State>::Input,
    ) -> std::result::Result<<C as AccessCapabilityContract>::Intent, PreparationError>
    + Send
    + Sync;
type AccessInterpreter<S, C> = dyn Fn(
        <S as State>::Input,
        &<C as AccessCapabilityContract>::Evidence,
    ) -> ProposedStateOutcome<<S as State>::Output, <S as State>::Failure>
    + Send
    + Sync;
/// Opaque Send future returned by one consuming qualified adapter ingress.
#[doc(hidden)]
pub type AccessIngressFuture<S, C> = BoxFuture<Result<AccessResolution<S, C>>>;
type AccessExecutor<S, C> = dyn Fn(CommittedCall<S, C>) -> AccessIngressFuture<S, C> + Send + Sync;

/// Pure State implementation whose callback receives only the exact typed input.
pub struct PureImplementation<S: State> {
    evaluate: Arc<PureEvaluator<S>>,
}

impl<S: State> Clone for PureImplementation<S> {
    fn clone(&self) -> Self {
        Self {
            evaluate: Arc::clone(&self.evaluate),
        }
    }
}

impl<S: State> PureImplementation<S> {
    /// Constructs one deterministic Pure implementation.
    pub fn new<F>(evaluate: F) -> Self
    where
        F: Fn(S::Input) -> ProposedStateOutcome<S::Output, S::Failure> + Send + Sync + 'static,
    {
        Self {
            evaluate: Arc::new(evaluate),
        }
    }

    /// Consumes one exact input without history or ambient I/O.
    pub fn evaluate(&self, input: S::Input) -> ProposedStateOutcome<S::Output, S::Failure> {
        (self.evaluate)(input)
    }

    /// Consumes one exact input while converting a callback panic into a redacted Runtime error.
    pub fn evaluate_contained(
        &self,
        input: S::Input,
    ) -> Result<ProposedStateOutcome<S::Output, S::Failure>> {
        catch_unwind(AssertUnwindSafe(|| (self.evaluate)(input)))
            .map_err(|_| RuntimeError::Unresolved)
    }
}

/// Access State implementation with borrowed preparation and consuming evidence interpretation.
pub struct AccessImplementation<S: State, C: AccessCapabilityContract> {
    prepare: Arc<AccessPreparer<S, C>>,
    interpret: Arc<AccessInterpreter<S, C>>,
}

impl<S: State, C: AccessCapabilityContract> Clone for AccessImplementation<S, C> {
    fn clone(&self) -> Self {
        Self {
            prepare: Arc::clone(&self.prepare),
            interpret: Arc::clone(&self.interpret),
        }
    }
}

impl<S: State, C: AccessCapabilityContract> AccessImplementation<S, C> {
    /// Constructs one access implementation.
    pub fn new<P, I>(prepare: P, interpret: I) -> Self
    where
        P: Fn(&S::Input) -> std::result::Result<C::Intent, PreparationError>
            + Send
            + Sync
            + 'static,
        I: Fn(S::Input, &C::Evidence) -> ProposedStateOutcome<S::Output, S::Failure>
            + Send
            + Sync
            + 'static,
    {
        Self {
            prepare: Arc::new(prepare),
            interpret: Arc::new(interpret),
        }
    }

    /// Borrows the exact cumulative input and produces one canonical intent.
    pub fn prepare(&self, input: &S::Input) -> std::result::Result<C::Intent, PreparationError> {
        (self.prepare)(input)
    }

    /// Consumes the State input and accepted authenticated evidence into one proposed outcome.
    pub fn interpret(
        &self,
        input: S::Input,
        evidence: &C::Evidence,
    ) -> ProposedStateOutcome<S::Output, S::Failure> {
        (self.interpret)(input, evidence)
    }

    /// Runs interpretation while converting a callback panic into a redacted Runtime error.
    pub fn interpret_contained(
        &self,
        input: S::Input,
        evidence: &C::Evidence,
    ) -> Result<ProposedStateOutcome<S::Output, S::Failure>> {
        catch_unwind(AssertUnwindSafe(|| (self.interpret)(input, evidence)))
            .map_err(|_| RuntimeError::Unresolved)
    }
}

/// One call-correlation value created only from a newly committed preparation.
pub struct CommittedCall<S: State, C: AccessCapabilityContract> {
    assembly_brand: Arc<RuntimeAssemblyBrand>,
    program_ref: ContentRef,
    store_scope_id: StoreScopeId,
    store_epoch: StoreEpoch,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
    occurrence: SequentialControlAddress,
    preparation: PreparationRef,
    call_id: StableId,
    input: QualifiedTypedValue<S::Input>,
    intent: C::Intent,
    binding: BindingDescriptor,
    execution_binding_ref: ContentRef,
    fact_continuation: Option<FactContinuation>,
    adapter: Arc<QualifiedAdapter<S, C>>,
}

impl<S: State, C: AccessCapabilityContract> CommittedCall<S, C> {
    /// Returns the immutable Program identity sealed into this call.
    pub const fn program_ref(&self) -> &ContentRef {
        &self.program_ref
    }

    /// Returns the Store scope sealed into this call.
    pub const fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }

    /// Returns the writer epoch sealed into this call.
    pub const fn store_epoch(&self) -> StoreEpoch {
        self.store_epoch
    }

    /// Returns the fixed tenant partition sealed into this call.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the run identity bound to this call.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact sequential occurrence.
    pub const fn occurrence(&self) -> &SequentialControlAddress {
        &self.occurrence
    }

    /// Returns the preparation identity bound to this call.
    pub const fn preparation(&self) -> &PreparationRef {
        &self.preparation
    }

    /// Returns the process-local call correlation identity.
    pub const fn call_id(&self) -> &StableId {
        &self.call_id
    }

    /// Borrows the exact cumulative input retained by this affine owner.
    pub const fn input(&self) -> &QualifiedTypedValue<S::Input> {
        &self.input
    }

    /// Borrows the canonical capability intent.
    pub const fn intent(&self) -> &C::Intent {
        &self.intent
    }

    /// Returns the exact canonical intent bytes fixed by the committed preparation.
    pub fn intent_canonical_bytes(&self) -> Result<Vec<u8>> {
        canonical_value(&self.intent)
            .map(|canonical| canonical.as_bytes().to_vec())
            .map_err(|_| RuntimeError::Value)
    }

    /// Borrows immutable binding evidence for adapter request derivation.
    pub const fn binding(&self) -> &BindingDescriptor {
        &self.binding
    }

    /// Returns the exact immutable execution-binding identity.
    pub const fn execution_binding_ref(&self) -> &ContentRef {
        &self.execution_binding_ref
    }

    /// Returns the Store-selected fact response, if this State declared prior facts.
    pub fn fact_selection(&self) -> Option<&mfm_facts::FactSelection> {
        self.fact_continuation
            .as_ref()
            .map(FactContinuation::selection)
    }

    /// Returns the fixed prior-fact request identity, if this State declared prior facts.
    pub fn fact_request(&self) -> Option<&mfm_journal::single_trust::ValueRef> {
        self.fact_continuation
            .as_ref()
            .map(FactContinuation::request)
    }

    /// Consumes the call and returns its typed input to the exact State owner.
    pub fn into_input(self) -> QualifiedTypedValue<S::Input> {
        self.input
    }

    /// Consumes the call into one adapter-authenticated, call-correlated evidence owner.
    pub fn accept_evidence(self, evidence: C::Evidence) -> Result<AcceptedOutcomeAccess<S, C>> {
        let Self {
            assembly_brand,
            input,
            call_id,
            intent,
            preparation,
            fact_continuation,
            ..
        } = self;
        AcceptedOutcomeAccess::new(
            assembly_brand,
            input,
            intent,
            evidence,
            call_id,
            preparation,
            fact_continuation,
        )
    }

    /// Consumes the call into an integrity-blocking, call-correlated evidence owner.
    pub fn accept_integrity(self, evidence: C::Evidence) -> Result<AcceptedIntegrityAccess<S, C>> {
        let Self {
            assembly_brand,
            input,
            call_id,
            intent,
            preparation,
            fact_continuation,
            ..
        } = self;
        AcceptedIntegrityAccess::new(
            assembly_brand,
            input,
            intent,
            evidence,
            call_id,
            preparation,
            fact_continuation,
        )
    }

    /// Consumes the call into an unresolved owner without exposing provider diagnostics.
    pub fn unresolved(self, classification: UnresolvedClassification) -> UnresolvedAccess<S, C> {
        let Self {
            assembly_brand,
            input,
            call_id,
            intent,
            preparation,
            fact_continuation,
            ..
        } = self;
        UnresolvedAccess::new(
            assembly_brand,
            input,
            intent,
            call_id,
            preparation,
            fact_continuation,
            classification,
        )
    }

    /// Consumes this call through the exact adapter sealed into the Runtime assembly.
    ///
    /// The caller supplies no target, binding, request, signer, or provider handle.  The
    /// adapter derives its request from this call's canonical intent and can only be entered
    /// once because the call itself is consumed.
    pub fn invoke_bound_adapter(self) -> AccessIngressFuture<S, C> {
        let adapter = Arc::clone(&self.adapter);
        adapter.enter(self)
    }
}

/// Preparation owner retaining the typed input until Store direct-new succeeds.
pub struct PreparedExecution<S: State, C: AccessCapabilityContract> {
    assembly_brand: Arc<RuntimeAssemblyBrand>,
    program_ref: ContentRef,
    run_id: RunId,
    occurrence: SequentialControlAddress,
    input: QualifiedTypedValue<S::Input>,
    intent: C::Intent,
    binding: BindingDescriptor,
    execution_binding_ref: ContentRef,
    _mode: PhantomData<C::Mode>,
}

/// Result of consuming a preparation owner through an opened semantic Store.
#[allow(clippy::large_enum_variant)]
pub enum OpenedPreparationCommit<S: State, C: AccessCapabilityContract> {
    /// The preparation crossed Store direct-new and the call is now eligible for adapter entry.
    Direct {
        /// The inert committed call.
        call: CommittedCall<S, C>,
        /// The selected prefix including the durable preparation.
        selected: SelectedRun,
    },
    /// Store returned a non-new disposition; the exact inert owner remains available for
    /// acknowledgement resolution or semantic rebind.
    Retained {
        /// The original preparation owner, including its typed input and intent.
        owner: PreparedExecution<S, C>,
        /// The unchanged selected predecessor.
        selected: SelectedRun,
        /// The known Store disposition that prevented direct call minting.
        disposition: AppendDisposition,
    },
    /// The append owner could not cross a known boundary; the exact inert owner remains retained.
    Rejected {
        /// The original preparation owner.
        owner: PreparedExecution<S, C>,
        /// The unchanged selected predecessor.
        selected: SelectedRun,
        /// Redacted failure classification.
        error: RuntimeError,
    },
}

impl<S: State, C: AccessCapabilityContract> PreparedExecution<S, C> {
    /// Creates a preparation by borrowing input while retaining the consuming typed value.
    pub(crate) fn new(
        assembly: &RuntimeAssembly,
        document: &ProgramDocument,
        program_ref: ContentRef,
        run_id: RunId,
        occurrence: SequentialControlAddress,
        input: QualifiedTypedValue<S::Input>,
    ) -> Result<Self> {
        C::validate().map_err(|_| RuntimeError::Mode)?;
        if !input.belongs_to_catalog(assembly.catalog()) {
            return Err(RuntimeError::Identity);
        }
        let Some(mfm_program::Declaration::State(state)) = document.declaration(&occurrence) else {
            return Err(RuntimeError::Identity);
        };
        if input.contract_ref() != state.input_contract_ref() {
            return Err(RuntimeError::Identity);
        }
        let binding = state.execution_binding().ok_or(RuntimeError::Mode)?.clone();
        let execution_binding_ref = binding.content_ref().map_err(|_| RuntimeError::Identity)?;
        let implementation = assembly.access_implementation::<S, C>()?;
        let intent = catch_unwind(AssertUnwindSafe(|| implementation.prepare(input.as_ref())))
            .map_err(|_| RuntimeError::Preparation)?
            .map_err(|_| RuntimeError::Preparation)?;
        Ok(Self {
            assembly_brand: Arc::clone(&assembly.brand),
            program_ref,
            run_id,
            occurrence,
            input,
            intent,
            binding,
            execution_binding_ref,
            _mode: PhantomData,
        })
    }

    /// Borrows the canonical intent before Store entry.
    pub const fn intent(&self) -> &C::Intent {
        &self.intent
    }

    /// Consumes this owner through the branded asynchronous Store boundary.
    pub async fn commit_opened(
        self,
        assembly: &RuntimeAssembly,
        store: &mfm_store::QualifiedHistoryPort,
        selected: SelectedRun,
    ) -> OpenedPreparationCommit<S, C> {
        if !Arc::ptr_eq(&self.assembly_brand, &assembly.brand) {
            return OpenedPreparationCommit::Rejected {
                owner: self,
                selected,
                error: RuntimeError::Identity,
            };
        }
        if self.program_ref != *selected.program_ref() {
            return OpenedPreparationCommit::Rejected {
                owner: self,
                selected,
                error: RuntimeError::Identity,
            };
        }
        let expected_intent_contract = match mfm_program::nominal_contract_ref::<C::Intent>() {
            Ok(value) => value,
            Err(_) => {
                return OpenedPreparationCommit::Rejected {
                    owner: self,
                    selected,
                    error: RuntimeError::Value,
                }
            }
        };
        let canonical_intent = match canonical_value(&self.intent) {
            Ok(value) => value,
            Err(_) => {
                return OpenedPreparationCommit::Rejected {
                    owner: self,
                    selected,
                    error: RuntimeError::Value,
                }
            }
        };
        let qualified_intent = match assembly
            .catalog()
            .qualify_retained::<C::Intent>(expected_intent_contract, canonical_intent.as_bytes())
        {
            Ok(value) => value,
            Err(_) => {
                return OpenedPreparationCommit::Rejected {
                    owner: self,
                    selected,
                    error: RuntimeError::Value,
                }
            }
        };
        if self.input.contract_ref() != selected.latest_context().contract_ref()
            || self.input.value_ref() != selected.latest_context().value_ref()
        {
            return OpenedPreparationCommit::Rejected {
                owner: self,
                selected,
                error: RuntimeError::Value,
            };
        }
        let adapter = match assembly.qualified_adapter::<S, C>(&self.binding) {
            Ok(adapter) => Arc::new(adapter),
            Err(error) => {
                return OpenedPreparationCommit::Rejected {
                    owner: self,
                    selected,
                    error,
                }
            }
        };
        let qualified_intent = qualified_intent.erase();
        let outcome = store
            .prepare_selected_access(selected, &qualified_intent)
            .await;
        let (selected, preparation, fact_continuation) = match outcome {
            mfm_store::AccessPreparationOutcome::Committed {
                selected,
                preparation,
                fact_continuation,
            } => (selected, preparation, fact_continuation),
            mfm_store::AccessPreparationOutcome::Retained {
                selected,
                disposition,
            } => {
                return OpenedPreparationCommit::Retained {
                    owner: self,
                    selected,
                    disposition,
                }
            }
            mfm_store::AccessPreparationOutcome::Rejected { selected, error: _ } => {
                return OpenedPreparationCommit::Rejected {
                    owner: self,
                    selected,
                    error: RuntimeError::Preparation,
                }
            }
        };
        let call_id = match mint_call_id(&self.run_id, &preparation) {
            Ok(call_id) => call_id,
            Err(error) => {
                return OpenedPreparationCommit::Rejected {
                    owner: self,
                    selected,
                    error,
                }
            }
        };
        let Self {
            assembly_brand,
            program_ref,
            run_id,
            occurrence,
            input,
            intent,
            binding,
            execution_binding_ref,
            ..
        } = self;
        OpenedPreparationCommit::Direct {
            call: CommittedCall {
                assembly_brand,
                program_ref,
                store_scope_id: store.identity().scope().clone(),
                store_epoch: store.identity().epoch(),
                tenant_scope_id: store.identity().tenant().clone(),
                run_id,
                occurrence,
                preparation,
                call_id,
                input,
                intent,
                binding,
                execution_binding_ref,
                fact_continuation,
                adapter,
            },
            selected,
        }
    }
}

/// An inert qualified adapter.  Its provider operation is unreachable without a committed call.
pub struct QualifiedAdapter<S: State, C: AccessCapabilityContract> {
    binding_ref: ContentRef,
    assembly_brand: Arc<RuntimeAssemblyBrand>,
    invoke: Arc<AccessExecutor<S, C>>,
}

impl<S: State, C: AccessCapabilityContract> QualifiedAdapter<S, C> {
    fn from_arc(
        assembly: &RuntimeAssembly,
        binding_ref: ContentRef,
        invoke: Arc<AccessExecutor<S, C>>,
    ) -> Self {
        Self {
            binding_ref,
            assembly_brand: Arc::clone(&assembly.brand),
            invoke,
        }
    }

    /// Returns the exact binding identity.
    pub const fn binding_ref(&self) -> &ContentRef {
        &self.binding_ref
    }

    /// Enters the provider-owned path with the one consuming committed call.
    pub fn enter(&self, call: CommittedCall<S, C>) -> AccessIngressFuture<S, C> {
        if !Arc::ptr_eq(&self.assembly_brand, &call.assembly_brand)
            || call.execution_binding_ref() != &self.binding_ref
        {
            return Box::pin(async { Err(RuntimeError::Identity) });
        }
        let future = match catch_unwind(AssertUnwindSafe(|| (self.invoke)(call))) {
            Ok(future) => future,
            Err(_) => return Box::pin(async { Err(RuntimeError::Unresolved) }),
        };
        Box::pin(PanicContainedFuture { inner: future })
    }
}

struct PanicContainedFuture<S: State, C: AccessCapabilityContract> {
    inner: AccessIngressFuture<S, C>,
}

impl<S: State, C: AccessCapabilityContract> Unpin for PanicContainedFuture<S, C> {}

impl<S: State, C: AccessCapabilityContract> Future for PanicContainedFuture<S, C> {
    type Output = Result<AccessResolution<S, C>>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| self.inner.as_mut().poll(context))) {
            Ok(result) => result,
            Err(_) => Poll::Ready(Err(RuntimeError::Unresolved)),
        }
    }
}

/// Immutable process assembly brand shared by callback-free consumers.
#[derive(Debug)]
pub(crate) struct RuntimeAssemblyBrand;

pub(crate) struct RegisteredState {
    pub(crate) state_implementation_ref: ContentRef,
    pub(crate) state_type: TypeId,
    pub(crate) capability_type: Option<TypeId>,
    pub(crate) input_contract_ref: ContentRef,
    pub(crate) output_contract_ref: ContentRef,
    pub(crate) failure_contract_ref: ContentRef,
    pub(crate) implementation: Box<dyn Any + Send + Sync>,
    pub(crate) dynamic: Arc<dyn crate::lifecycle::DynamicStateRegistration>,
}

struct RegisteredAdapter {
    binding: BindingDescriptor,
    binding_ref: ContentRef,
    state_implementation_ref: ContentRef,
    state_type: TypeId,
    capability_type: TypeId,
    invoke: Box<dyn Any + Send + Sync>,
}

struct ProcessRegistry {
    states: Vec<RegisteredState>,
    adapters: Vec<RegisteredAdapter>,
}

/// Builder for one immutable catalog-wide Runtime registry.
pub struct RuntimeAssemblyBuilder {
    catalog: ProgramCatalog,
    registrations: Vec<RegisteredState>,
    adapters: Vec<RegisteredAdapter>,
    brand: Arc<RuntimeAssemblyBrand>,
}

impl RuntimeAssemblyBuilder {
    /// Starts an assembly builder for one exact catalog.
    pub fn new(catalog: ProgramCatalog) -> Self {
        Self {
            catalog,
            registrations: Vec::new(),
            adapters: Vec::new(),
            brand: Arc::new(RuntimeAssemblyBrand),
        }
    }

    fn ensure_unique(&self, state_implementation_ref: &ContentRef) -> Result<()> {
        if self
            .registrations
            .iter()
            .any(|registered| registered.state_implementation_ref == *state_implementation_ref)
        {
            Err(RuntimeError::Identity)
        } else {
            Ok(())
        }
    }

    /// Registers one deterministic semantic State implementation.
    pub fn register_pure<S: State>(&mut self, implementation: PureImplementation<S>) -> Result<()> {
        let state_implementation_ref = state_implementation_ref::<S>()?;
        self.ensure_unique(&state_implementation_ref)?;
        let input_contract_ref = mfm_program::nominal_contract_ref::<S::Input>()?;
        let output_contract_ref = mfm_program::nominal_contract_ref::<S::Output>()?;
        let failure_contract_ref = mfm_program::nominal_contract_ref::<S::Failure>()?;
        if !(self.catalog.contains_value::<S::Input>(&input_contract_ref)
            && self
                .catalog
                .contains_value::<S::Output>(&output_contract_ref)
            && self
                .catalog
                .contains_value::<S::Failure>(&failure_contract_ref))
        {
            return Err(RuntimeError::Identity);
        }
        let dynamic = crate::lifecycle::pure_registration(implementation.clone());
        self.registrations.push(RegisteredState {
            state_implementation_ref,
            state_type: TypeId::of::<S>(),
            capability_type: None,
            input_contract_ref,
            output_contract_ref,
            failure_contract_ref,
            implementation: Box::new(implementation),
            dynamic,
        });
        Ok(())
    }

    /// Registers one semantic Access State implementation.
    pub fn register_access<S: State, C: AccessCapabilityContract>(
        &mut self,
        implementation: AccessImplementation<S, C>,
    ) -> Result<()> {
        C::validate().map_err(|_| RuntimeError::Mode)?;
        let state_implementation_ref = state_implementation_ref::<S>()?;
        self.ensure_unique(&state_implementation_ref)?;
        let capability_contract_ref = mfm_program::capability_contract_ref::<C>()?;
        if !self
            .catalog
            .contains_capability::<C>(&capability_contract_ref)
        {
            return Err(RuntimeError::Identity);
        }
        let input_contract_ref = mfm_program::nominal_contract_ref::<S::Input>()?;
        let output_contract_ref = mfm_program::nominal_contract_ref::<S::Output>()?;
        let failure_contract_ref = mfm_program::nominal_contract_ref::<S::Failure>()?;
        if !(self.catalog.contains_value::<S::Input>(&input_contract_ref)
            && self
                .catalog
                .contains_value::<S::Output>(&output_contract_ref)
            && self
                .catalog
                .contains_value::<S::Failure>(&failure_contract_ref))
        {
            return Err(RuntimeError::Identity);
        }
        let dynamic = crate::lifecycle::access_registration(implementation.clone());
        self.registrations.push(RegisteredState {
            state_implementation_ref,
            state_type: TypeId::of::<S>(),
            capability_type: Some(TypeId::of::<C>()),
            input_contract_ref,
            output_contract_ref,
            failure_contract_ref,
            implementation: Box::new(implementation),
            dynamic,
        });
        Ok(())
    }

    /// Registers one exact adapter invocation for an Access binding descriptor.
    pub fn register_adapter<S: State, C: AccessCapabilityContract, F>(
        &mut self,
        binding: BindingDescriptor,
        invoke: F,
    ) -> Result<()>
    where
        F: Fn(CommittedCall<S, C>) -> AccessIngressFuture<S, C> + Send + Sync + 'static,
    {
        C::validate().map_err(|_| RuntimeError::Mode)?;
        let state_implementation_ref = state_implementation_ref::<S>()?;
        let capability_contract_ref = mfm_program::capability_contract_ref::<C>()?;
        let registered = self
            .registrations
            .iter()
            .find(|registered| {
                registered.state_implementation_ref == state_implementation_ref
                    && registered.state_type == TypeId::of::<S>()
                    && registered.capability_type == Some(TypeId::of::<C>())
            })
            .ok_or(RuntimeError::Identity)?;
        if binding.state_implementation_ref() != &state_implementation_ref
            || binding.capability_contract_ref() != Some(&capability_contract_ref)
            || binding.adapter_implementation_ref().is_none()
            || registered.capability_type.is_none()
        {
            return Err(RuntimeError::Identity);
        }
        let binding_ref = binding.content_ref().map_err(|_| RuntimeError::Identity)?;
        if self.adapters.iter().any(|registered| {
            registered.binding_ref == binding_ref
                || (registered.binding == binding
                    && (registered.state_type != TypeId::of::<S>()
                        || registered.capability_type != TypeId::of::<C>()))
        }) {
            return Err(RuntimeError::Identity);
        }
        self.adapters.push(RegisteredAdapter {
            binding,
            binding_ref,
            state_implementation_ref,
            state_type: TypeId::of::<S>(),
            capability_type: TypeId::of::<C>(),
            invoke: Box::new(Arc::new(invoke) as Arc<AccessExecutor<S, C>>),
        });
        Ok(())
    }

    /// Finishes the immutable catalog-wide assembly.
    pub fn finish(self) -> Result<RuntimeAssembly> {
        Ok(RuntimeAssembly {
            catalog: self.catalog,
            registry: ProcessRegistry {
                states: self.registrations,
                adapters: self.adapters,
            },
            brand: self.brand,
        })
    }
}

/// Immutable process assembly brand shared by callback-free consumers.
pub struct RuntimeAssembly {
    catalog: ProgramCatalog,
    registry: ProcessRegistry,
    brand: Arc<RuntimeAssemblyBrand>,
}

impl RuntimeAssembly {
    /// Returns the exact callback-free Program catalog.
    pub const fn catalog(&self) -> &ProgramCatalog {
        &self.catalog
    }

    pub(crate) fn has_brand(&self, brand: &Arc<RuntimeAssemblyBrand>) -> bool {
        Arc::ptr_eq(&self.brand, brand)
    }

    pub(crate) fn dynamic_registration(
        &self,
        state_implementation_ref: &ContentRef,
    ) -> Result<Arc<dyn crate::lifecycle::DynamicStateRegistration>> {
        self.registry
            .states
            .iter()
            .find(|registered| &registered.state_implementation_ref == state_implementation_ref)
            .map(|registered| Arc::clone(&registered.dynamic))
            .ok_or(RuntimeError::Identity)
    }

    pub(crate) fn reify_value(
        &self,
        contract: &ContentRef,
        canonical_bytes: &[u8],
    ) -> Result<crate::lifecycle::ErasedValue> {
        self.catalog
            .qualify_retained_erased(contract.clone(), canonical_bytes)
            .map_err(|_| RuntimeError::Value)
    }

    /// Returns one registered Pure implementation under its State-derived identity.
    pub fn pure_implementation<S: State>(&self) -> Result<&PureImplementation<S>> {
        let state_implementation_ref = state_implementation_ref::<S>()?;
        self.registry
            .states
            .iter()
            .find(|registered| {
                registered.state_implementation_ref == state_implementation_ref
                    && registered.state_type == TypeId::of::<S>()
                    && registered.capability_type.is_none()
            })
            .and_then(|registered| registered.implementation.downcast_ref())
            .ok_or(RuntimeError::Identity)
    }

    /// Returns one registered Access implementation under its State-derived identity.
    pub fn access_implementation<S: State, C: AccessCapabilityContract>(
        &self,
    ) -> Result<&AccessImplementation<S, C>> {
        let state_implementation_ref = state_implementation_ref::<S>()?;
        self.registry
            .states
            .iter()
            .find(|registered| {
                registered.state_implementation_ref == state_implementation_ref
                    && registered.state_type == TypeId::of::<S>()
                    && registered.capability_type == Some(TypeId::of::<C>())
            })
            .and_then(|registered| registered.implementation.downcast_ref())
            .ok_or(RuntimeError::Identity)
    }

    /// Reifies one inert adapter from the exact registered Access binding.
    pub fn qualified_adapter<S: State, C: AccessCapabilityContract>(
        &self,
        binding: &BindingDescriptor,
    ) -> Result<QualifiedAdapter<S, C>> {
        let state_implementation_ref = state_implementation_ref::<S>()?;
        let binding_ref = binding.content_ref().map_err(|_| RuntimeError::Identity)?;
        let registered = self
            .registry
            .adapters
            .iter()
            .find(|registered| {
                registered.binding_ref == binding_ref
                    && registered.binding == *binding
                    && registered.state_implementation_ref == state_implementation_ref
                    && registered.state_type == TypeId::of::<S>()
                    && registered.capability_type == TypeId::of::<C>()
            })
            .ok_or(RuntimeError::Identity)?;
        let invoke = registered
            .invoke
            .downcast_ref::<Arc<AccessExecutor<S, C>>>()
            .ok_or(RuntimeError::Identity)?;
        Ok(QualifiedAdapter::from_arc(
            self,
            binding_ref,
            Arc::clone(invoke),
        ))
    }

    pub(crate) fn validate_program(&self, program: &Program) -> Result<()> {
        if !program.belongs_to_catalog(&self.catalog) {
            return Err(RuntimeError::Identity);
        }
        for declaration in program.document().declarations() {
            let mfm_program::Declaration::State(state) = declaration else {
                continue;
            };
            let registered = self
                .registry
                .states
                .iter()
                .find(|registered| {
                    registered.state_implementation_ref == *state.state_implementation_ref()
                })
                .ok_or(RuntimeError::Identity)?;
            if registered.input_contract_ref != *state.input_contract_ref()
                || registered.output_contract_ref != *state.output_contract_ref()
                || state.failure_contract_ref() != Some(&registered.failure_contract_ref)
            {
                return Err(RuntimeError::Identity);
            }
            match state.execution() {
                mfm_program::ExecutionMode::Pure if registered.capability_type.is_none() => {}
                mfm_program::ExecutionMode::Read {
                    capability_contract_ref,
                    ..
                }
                | mfm_program::ExecutionMode::Effect {
                    capability_contract_ref,
                    ..
                } if registered.capability_type.is_some() => {
                    let binding = state.execution_binding().ok_or(RuntimeError::Identity)?;
                    let binding_ref = binding.content_ref().map_err(|_| RuntimeError::Identity)?;
                    if binding.capability_contract_ref() != Some(capability_contract_ref) {
                        return Err(RuntimeError::Identity);
                    }
                    let adapter = self
                        .registry
                        .adapters
                        .iter()
                        .find(|adapter| {
                            adapter.binding_ref == binding_ref
                                && adapter.binding == *binding
                                && adapter.state_implementation_ref
                                    == *state.state_implementation_ref()
                                && Some(adapter.capability_type) == registered.capability_type
                        })
                        .ok_or(RuntimeError::Identity)?;
                    let _ = adapter;
                }
                _ => return Err(RuntimeError::Mode),
            }
        }
        Ok(())
    }
}

fn mint_call_id(run_id: &RunId, preparation: &PreparationRef) -> Result<StableId> {
    StableId::new(format!(
        "call/{}/{}/{}",
        short_stable_id_fragment(run_id.as_str(), 96),
        preparation.run_sequence(),
        preparation.record_ordinal()
    ))
    .map_err(|_| RuntimeError::Identity)
}
