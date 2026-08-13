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

use mfm_canonical::raw_content_digest;
use mfm_capabilities::{
    AccessCapabilityContract, AccessMode, EffectEntryMode, EffectMode, ProposedStateOutcome,
    ReadMode,
};
use mfm_ids::{
    short_stable_id_fragment, AppendRequestId, ContentRef, DigestAlgorithm, DigestBytes, RunId,
    SchemaId, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::single_trust::{
    BindingDescriptor, ImmutableObject, PreparationMode, PreparationRef, SequentialControlAddress,
    StatePrepared,
};
use mfm_program::{canonical_value, Program, ProgramCatalog, QualifiedTypedValue};
use mfm_store::single_trust::{
    AppendDisposition, FactContinuation, PreparationAppend, PreparedConclusion,
    Result as StoreResult, RunStore,
};
use mfm_values::MfmValue;

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

/// A callback-free typed State contract.
pub trait State: Send + Sync + 'static {
    /// Complete cumulative input consumed by this State.
    type Input: MfmValue;
    /// Complete successor context or terminal public result.
    type Output: MfmValue;
    /// Explicit fail-fast domain value.
    type Failure: MfmValue;

    /// Returns the stable State implementation identity.
    fn state_id() -> Result<StableId>;
}

/// Marker for deterministic State evaluation.
pub struct Pure;

/// Marker for a non-mutating capability State.
pub struct Read<C: AccessCapabilityContract>(PhantomData<fn() -> C>);

/// Marker for a mutating capability State.
pub struct Effect<C: AccessCapabilityContract>(PhantomData<fn() -> C>);

/// Maps a sealed capability mode to its journal preparation mode.
pub trait RuntimePreparationMode: AccessMode {
    /// Returns the fixed journal mode for one capability's attempt bound.
    fn journal_mode(total_attempt_bound: std::num::NonZeroU16) -> PreparationMode;
}

impl RuntimePreparationMode for ReadMode {
    fn journal_mode(total_attempt_bound: std::num::NonZeroU16) -> PreparationMode {
        PreparationMode::Read {
            total_attempt_bound: total_attempt_bound.get(),
        }
    }
}

impl<E: EffectEntryMode> RuntimePreparationMode for EffectMode<E> {
    fn journal_mode(total_attempt_bound: std::num::NonZeroU16) -> PreparationMode {
        PreparationMode::Effect {
            total_attempt_bound: total_attempt_bound.get(),
            absorbing: E::ABSORBING,
        }
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

    /// Consumes accepted evidence into one call-bound conclusion proposal.
    pub fn conclude(
        self,
        outcome: ProposedStateOutcome<S::Output, S::Failure>,
    ) -> AccessHandlerResolution<S, S::Output, S::Failure, C> {
        AccessHandlerResolution {
            assembly_brand: self.assembly_brand,
            input: self.input,
            call_id: self.call_id,
            intent: self.intent,
            evidence: Some(self.evidence),
            outcome: Some(outcome),
            classification: None,
            preparation: self.preparation,
            fact_continuation: self.fact_continuation,
        }
    }
}

/// Affine evidence for a capability-specific integrity block.
///
/// The caller must supply the State's declared typed failure.  The wrapper has no success
/// constructor and therefore cannot accidentally grant a successor context or retry authority.
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

    /// Consumes integrity evidence into the exact declared typed failure route.
    pub fn conclude_blocked(
        self,
        failure: S::Failure,
    ) -> AccessHandlerResolution<S, S::Output, S::Failure, C> {
        AccessHandlerResolution {
            assembly_brand: self.assembly_brand,
            input: self.input,
            call_id: self.call_id,
            intent: self.intent,
            evidence: Some(self.evidence),
            outcome: Some(ProposedStateOutcome::Failure(failure)),
            classification: None,
            preparation: self.preparation,
            fact_continuation: self.fact_continuation,
        }
    }
}

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

    /// Consumes the unresolved owner into a closed handler result.  Runtime will keep the
    /// preparation neutral and will not pass this branch to conclusion qualification.
    pub fn finish(self) -> AccessHandlerResolution<S, S::Output, S::Failure, C> {
        AccessHandlerResolution {
            assembly_brand: self.assembly_brand,
            input: self.input,
            call_id: self.call_id,
            intent: self.intent,
            evidence: None,
            outcome: None,
            classification: Some(self.classification),
            preparation: self.preparation,
            fact_continuation: self.fact_continuation,
        }
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

/// Opaque closed handler resolution owned by the Runtime handoff.
pub struct AccessHandlerResolution<S: State, O, F, C: AccessCapabilityContract> {
    assembly_brand: Arc<RuntimeAssemblyBrand>,
    input: QualifiedTypedValue<S::Input>,
    call_id: StableId,
    intent: C::Intent,
    evidence: Option<C::Evidence>,
    outcome: Option<ProposedStateOutcome<O, F>>,
    classification: Option<UnresolvedClassification>,
    preparation: PreparationRef,
    fact_continuation: Option<FactContinuation>,
}

impl<S: State, O, F, C: AccessCapabilityContract> AccessHandlerResolution<S, O, F, C> {
    /// Consumes the resolution into its exact call-bound components.
    #[allow(clippy::type_complexity)]
    pub(crate) fn into_parts(
        self,
    ) -> (
        Arc<RuntimeAssemblyBrand>,
        QualifiedTypedValue<S::Input>,
        StableId,
        C::Intent,
        Option<C::Evidence>,
        Option<ProposedStateOutcome<O, F>>,
        Option<UnresolvedClassification>,
        PreparationRef,
        Option<FactContinuation>,
    ) {
        (
            self.assembly_brand,
            self.input,
            self.call_id,
            self.intent,
            self.evidence,
            self.outcome,
            self.classification,
            self.preparation,
            self.fact_continuation,
        )
    }
}

type PureEvaluator<S> = dyn Fn(&<S as State>::Input) -> ProposedStateOutcome<<S as State>::Output, <S as State>::Failure>
    + Send
    + Sync;
type AccessPreparer<S, C> = dyn Fn(
        &<S as State>::Input,
    ) -> std::result::Result<<C as AccessCapabilityContract>::Intent, PreparationError>
    + Send
    + Sync;
/// Opaque Send future returned by one consuming State/adapter execution.
#[doc(hidden)]
pub type AccessResolutionFuture<S, C> =
    BoxFuture<Result<AccessHandlerResolution<S, <S as State>::Output, <S as State>::Failure, C>>>;
type AccessExecutor<S, C> =
    dyn Fn(CommittedCall<S, C>) -> AccessResolutionFuture<S, C> + Send + Sync;

/// Pure State implementation whose callback receives only the exact typed input.
pub struct PureImplementation<S: State> {
    evaluate: Arc<PureEvaluator<S>>,
}

impl<S: State> PureImplementation<S> {
    /// Constructs one deterministic Pure implementation.
    pub fn new<F>(evaluate: F) -> Self
    where
        F: Fn(&S::Input) -> ProposedStateOutcome<S::Output, S::Failure> + Send + Sync + 'static,
    {
        Self {
            evaluate: Arc::new(evaluate),
        }
    }

    /// Evaluates one exact input without history or ambient I/O.
    pub fn evaluate(&self, input: &S::Input) -> ProposedStateOutcome<S::Output, S::Failure> {
        (self.evaluate)(input)
    }

    /// Evaluates one exact input while converting a callback panic into a redacted Runtime error.
    pub fn evaluate_contained(
        &self,
        input: &S::Input,
    ) -> Result<ProposedStateOutcome<S::Output, S::Failure>> {
        catch_unwind(AssertUnwindSafe(|| (self.evaluate)(input)))
            .map_err(|_| RuntimeError::Unresolved)
    }
}

/// Access State implementation with separate borrowed preparation and consuming execution.
pub struct AccessImplementation<S: State, C: AccessCapabilityContract> {
    prepare: Arc<AccessPreparer<S, C>>,
    execute: Arc<AccessExecutor<S, C>>,
}

impl<S: State, C: AccessCapabilityContract> AccessImplementation<S, C> {
    /// Constructs one access implementation.  The execution closure cannot be called without a
    /// committed preparation token.
    pub fn new<P, E>(prepare: P, execute: E) -> Self
    where
        P: Fn(&S::Input) -> std::result::Result<C::Intent, PreparationError>
            + Send
            + Sync
            + 'static,
        E: Fn(CommittedCall<S, C>) -> AccessResolutionFuture<S, C> + Send + Sync + 'static,
    {
        Self {
            prepare: Arc::new(prepare),
            execute: Arc::new(execute),
        }
    }

    /// Borrows the exact cumulative input and produces one canonical intent.
    pub fn prepare(&self, input: &S::Input) -> std::result::Result<C::Intent, PreparationError> {
        (self.prepare)(input)
    }

    /// Consumes the one Runtime-minted call into the implementation future.
    pub fn execute(&self, call: CommittedCall<S, C>) -> AccessResolutionFuture<S, C> {
        (self.execute)(call)
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

    /// Returns the one-use preparation-bound fact identity, if this State declared prior facts.
    pub fn fact_selection(&self) -> Option<&mfm_journal::single_trust::ValueRef> {
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
    mode: PreparationMode,
    fact_request: Option<(mfm_journal::single_trust::ValueRef, ImmutableObject)>,
    fact_selection: Option<(mfm_journal::single_trust::ValueRef, ImmutableObject)>,
    _mode: PhantomData<C::Mode>,
}

impl<S: State, C: AccessCapabilityContract> PreparedExecution<S, C>
where
    C::Mode: RuntimePreparationMode,
{
    /// Creates a preparation by borrowing input while retaining the consuming typed value.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        assembly: &RuntimeAssembly,
        run_id: RunId,
        occurrence: SequentialControlAddress,
        input: QualifiedTypedValue<S::Input>,
        binding: BindingDescriptor,
        execution_binding_ref: ContentRef,
    ) -> Result<Self> {
        C::validate().map_err(|_| RuntimeError::Mode)?;
        if !input.belongs_to_catalog(assembly.catalog()) {
            return Err(RuntimeError::Identity);
        }
        let Some(mfm_program::Declaration::State(state)) =
            assembly.program().document().declaration(&occurrence)
        else {
            return Err(RuntimeError::Identity);
        };
        if input.contract_ref() != state.input_contract_ref()
            || input.value_ref() != &qualified_value_ref(input.as_ref())?
        {
            return Err(RuntimeError::Identity);
        }
        let capability_contract_ref = state
            .execution()
            .capability_contract_ref()
            .ok_or(RuntimeError::Mode)?;
        let implementation = assembly.access_implementation::<S, C>(
            state.state_implementation_ref(),
            capability_contract_ref,
            &execution_binding_ref,
        )?;
        let registered_adapter_ref = assembly
            .registry
            .states
            .iter()
            .find(|registered| {
                &registered.state_implementation_ref == state.state_implementation_ref()
                    && registered.binding_ref.as_ref() == Some(&execution_binding_ref)
                    && registered.capability_contract_ref.as_ref() == Some(capability_contract_ref)
            })
            .and_then(|registered| registered.adapter_implementation_ref.as_ref())
            .ok_or(RuntimeError::Identity)?;
        if input.contract_ref() != state.input_contract_ref()
            || state.execution_binding_ref() != Some(&execution_binding_ref)
            || binding.state_implementation_ref() != state.state_implementation_ref()
            || binding.capability_contract_ref() != Some(capability_contract_ref)
            || binding.adapter_implementation_ref() != Some(registered_adapter_ref)
            || binding.effect_domain() != state.effect_domain()
            || binding.content_ref().map_err(|_| RuntimeError::Identity)? != execution_binding_ref
            || binding.validate().is_err()
        {
            return Err(RuntimeError::Identity);
        }
        let intent = catch_unwind(AssertUnwindSafe(|| implementation.prepare(input.as_ref())))
            .map_err(|_| RuntimeError::Preparation)?
            .map_err(|_| RuntimeError::Preparation)?;
        let fact_request = catch_unwind(AssertUnwindSafe(|| C::prior_fact_selection(&intent)))
            .map_err(|_| RuntimeError::Preparation)?
            .map_err(|_| RuntimeError::Preparation)?
            .map(|request| -> Result<_> {
                let value_ref = qualified_value_ref(&request)?;
                let value =
                    mfm_journal::single_trust::ValueRef::new(value_ref.clone(), value_ref.clone());
                let object = value_object(&request, &value_ref)?;
                Ok((value, object))
            })
            .transpose()?;
        Ok(Self {
            assembly_brand: Arc::clone(&assembly.brand),
            program_ref: assembly.program_ref().clone(),
            run_id,
            occurrence,
            input,
            intent,
            binding,
            execution_binding_ref,
            mode: C::Mode::journal_mode(C::total_attempt_bound()),
            fact_request,
            fact_selection: None,
            _mode: PhantomData,
        })
    }

    /// Borrows the canonical intent before Store entry.
    pub const fn intent(&self) -> &C::Intent {
        &self.intent
    }

    /// Attaches one already-qualified prior-fact selection object to this exact preparation.
    ///
    /// The object is copied into the append closure; its bytes are still validated once by the
    /// Journal/Store ingress. A replacement must supply the new fixed selection explicitly.
    pub fn with_fact_selection(
        mut self,
        selection: mfm_journal::single_trust::ValueRef,
        object: ImmutableObject,
    ) -> Result<Self> {
        if !C::requires_prior_facts()
            || self.fact_request.is_none()
            || !selection.is_schema_bound()
            || object.content_ref() != selection.value_ref()
        {
            return Err(RuntimeError::Value);
        }
        self.fact_selection = Some((selection, object));
        Ok(self)
    }

    /// Consumes this owner and creates a `CommittedCall` only for a direct-new append.
    #[allow(clippy::too_many_arguments)]
    pub fn commit(
        self,
        assembly: &RuntimeAssembly,
        store: &RunStore,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        input_ref: mfm_journal::single_trust::ValueRef,
        intent_ref: mfm_journal::single_trust::ValueRef,
        maximum_conclusion_bytes: u64,
    ) -> Result<CommittedCall<S, C>> {
        self.commit_with_replacement(
            assembly,
            store,
            expected_sequence,
            append_request_id,
            input_ref,
            intent_ref,
            maximum_conclusion_bytes,
            0,
            None,
        )
    }

    /// Consumes this owner for a replacement preparation under the exact selected parent.
    #[allow(clippy::too_many_arguments)]
    pub fn commit_with_replacement(
        self,
        assembly: &RuntimeAssembly,
        store: &RunStore,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        input_ref: mfm_journal::single_trust::ValueRef,
        intent_ref: mfm_journal::single_trust::ValueRef,
        maximum_conclusion_bytes: u64,
        preparation_ordinal: u16,
        replaces: Option<PreparationRef>,
    ) -> Result<CommittedCall<S, C>> {
        if !Arc::ptr_eq(&self.assembly_brand, &assembly.brand) {
            return Err(RuntimeError::Identity);
        }
        let Self {
            assembly_brand,
            program_ref,
            run_id,
            occurrence,
            input,
            intent,
            binding,
            execution_binding_ref,
            mode,
            fact_request,
            fact_selection,
            ..
        } = self;
        if C::requires_prior_facts() != fact_request.is_some()
            || fact_request.is_some() != fact_selection.is_some()
        {
            return Err(RuntimeError::Preparation);
        }
        let intent_object_ref = intent_ref.value_ref().clone();
        let input_object_ref = input_ref.value_ref().clone();
        if input.contract_ref() != input_ref.contract_ref()
            || input.value_ref() != &input_object_ref
            || intent_ref.contract_ref() != &nominal_contract_ref::<C::Intent>()?
            || intent_ref.value_ref() != &qualified_value_ref(&intent)?
            || !intent_ref.is_schema_bound()
        {
            return Err(RuntimeError::Value);
        }
        let prepared = StatePrepared::new(
            occurrence.clone(),
            preparation_ordinal,
            input_ref,
            intent_ref,
            fact_request.as_ref().map(|(request, _)| request.clone()),
            fact_selection
                .as_ref()
                .map(|(selection, _)| selection.clone()),
            mode,
            binding.clone(),
            execution_binding_ref.clone(),
            replaces,
            maximum_conclusion_bytes,
        )
        .map_err(|_| RuntimeError::Preparation)?;
        let mut objects = vec![
            value_object(input.as_ref(), &input_object_ref)?,
            value_object(&intent, &intent_object_ref)?,
        ];
        if let Some((_, object)) = fact_request {
            objects.push(object);
        }
        if let Some((_, object)) = fact_selection {
            objects.push(object);
        }
        let append = store
            .prepare_access(
                &run_id,
                assembly.program().document(),
                expected_sequence,
                append_request_id,
                prepared,
                objects,
            )
            .map_err(|_| RuntimeError::Preparation)?;
        if !matches!(
            append.disposition(),
            AppendDisposition::NewlyCommitted { .. }
        ) {
            return Err(RuntimeError::PreparationNotCommitted);
        }
        let preparation = append
            .preparation()
            .cloned()
            .ok_or(RuntimeError::PreparationNotCommitted)?;
        let call_id = mint_call_id(&run_id, &preparation)?;
        let fact_continuation = append.into_fact_continuation();
        Ok(CommittedCall {
            assembly_brand,
            program_ref,
            store_scope_id: store.scope().clone(),
            store_epoch: store.epoch(),
            tenant_scope_id: store.tenant().clone(),
            run_id,
            occurrence,
            preparation,
            call_id,
            input,
            intent,
            binding,
            execution_binding_ref,
            fact_continuation,
        })
    }
}

/// Runtime-owned outer owner for a Store conclusion append and its already-typed successor.
pub struct PendingConclusion<T: MfmValue> {
    assembly_brand: Arc<RuntimeAssemblyBrand>,
    owner: PreparedConclusion,
    successor: QualifiedTypedValue<T>,
}

impl<T: MfmValue> PendingConclusion<T> {
    /// Constructs an owner around a Store-prepared conclusion.
    fn new(
        assembly: &RuntimeAssembly,
        owner: PreparedConclusion,
        successor: QualifiedTypedValue<T>,
    ) -> Result<Self> {
        if !successor.belongs_to_catalog(assembly.catalog())
            || !matches!(
                owner.frame().record(),
                mfm_journal::single_trust::RunRecord::StateConcluded(conclusion)
                    if matches!(
                        conclusion.outcome(),
                        mfm_journal::single_trust::StateOutcome::Success(value)
                            if value.contract_ref() == successor.contract_ref()
                                && value.value_ref() == successor.value_ref()
                    )
            )
        {
            return Err(RuntimeError::Identity);
        }
        Ok(Self {
            assembly_brand: Arc::clone(&assembly.brand),
            owner,
            successor,
        })
    }

    /// Returns whether this pending owner belongs to the exact assembly.
    pub fn belongs_to_assembly(&self, assembly: &RuntimeAssembly) -> bool {
        Arc::ptr_eq(&self.assembly_brand, &assembly.brand)
    }

    /// Consumes the pending owner, commits once, and releases the retained typed successor only
    /// after Store reports a durable or found-identical conclusion.
    pub fn commit(
        self,
        assembly: &RuntimeAssembly,
        store: &RunStore,
    ) -> Result<(AppendDisposition, QualifiedTypedValue<T>)> {
        if !self.belongs_to_assembly(assembly) {
            return Err(RuntimeError::Identity);
        }
        let Self {
            owner, successor, ..
        } = self;
        let disposition = owner.commit(store).map_err(|_| RuntimeError::Conclusion)?;
        Ok((disposition, successor))
    }
}

/// Runtime owner for a durable typed failure conclusion.
pub struct PendingFailure {
    assembly_brand: Arc<RuntimeAssemblyBrand>,
    owner: PreparedConclusion,
}

impl PendingFailure {
    /// Constructs one failure owner around a Store-prepared conclusion.
    fn new(assembly: &RuntimeAssembly, owner: PreparedConclusion) -> Result<Self> {
        if !matches!(
            owner.frame().record(),
            mfm_journal::single_trust::RunRecord::StateConcluded(conclusion)
                if matches!(
                    conclusion.outcome(),
                    mfm_journal::single_trust::StateOutcome::Failure(_)
                )
        ) {
            return Err(RuntimeError::Identity);
        }
        Ok(Self {
            assembly_brand: Arc::clone(&assembly.brand),
            owner,
        })
    }

    /// Returns whether this pending owner belongs to the exact assembly.
    pub fn belongs_to_assembly(&self, assembly: &RuntimeAssembly) -> bool {
        Arc::ptr_eq(&self.assembly_brand, &assembly.brand)
    }

    /// Commits the failure once and returns its mechanical disposition.
    pub fn commit(self, assembly: &RuntimeAssembly, store: &RunStore) -> Result<AppendDisposition> {
        if !self.belongs_to_assembly(assembly) {
            return Err(RuntimeError::Identity);
        }
        self.owner
            .commit(store)
            .map_err(|_| RuntimeError::Conclusion)
    }
}

/// Access conclusion owner returned after one exact accepted evidence interpretation.
#[allow(clippy::large_enum_variant)]
pub enum AccessConclusion<S: State> {
    /// A durable success retains the already-qualified successor context.
    Success(PendingConclusion<S::Output>),
    /// A durable failure retains no successor context or retry authority.
    Failure(PendingFailure),
}

/// Prepares one Pure success conclusion without invoking Store callbacks or re-decoding output.
#[allow(clippy::too_many_arguments)]
pub fn prepare_pure_success<S: State>(
    assembly: &RuntimeAssembly,
    store: &RunStore,
    run_id: &RunId,
    expected_sequence: u64,
    append_request_id: AppendRequestId,
    occurrence: SequentialControlAddress,
    successor: QualifiedTypedValue<S::Output>,
    output: mfm_journal::single_trust::ValueRef,
    objects: Vec<mfm_journal::single_trust::ImmutableObject>,
) -> Result<PendingConclusion<S::Output>> {
    if successor.contract_ref() != output.contract_ref()
        || successor.value_ref() != output.value_ref()
        || !successor.belongs_to_catalog(assembly.catalog())
    {
        return Err(RuntimeError::Value);
    }
    let mut objects = objects;
    objects.push(value_object(successor.as_ref(), output.value_ref())?);
    let owner = store
        .prepare_conclusion(
            run_id,
            assembly.program().document(),
            expected_sequence,
            append_request_id,
            mfm_journal::single_trust::StateConcluded::Pure {
                occurrence,
                outcome: mfm_journal::single_trust::StateOutcome::Success(output),
                fact_publication: None,
            },
            objects,
            mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
        )
        .map_err(|_| RuntimeError::Conclusion)?;
    PendingConclusion::new(assembly, owner, successor)
}

/// Prepares one Access success or failure conclusion from a call-bound opaque resolution.
#[allow(clippy::too_many_arguments)]
pub fn prepare_access_resolution<S: State, C: AccessCapabilityContract>(
    assembly: &RuntimeAssembly,
    catalog: &ProgramCatalog,
    store: &RunStore,
    run_id: &RunId,
    expected_sequence: u64,
    append_request_id: AppendRequestId,
    occurrence: SequentialControlAddress,
    preparation: PreparationRef,
    call_id: StableId,
    evidence_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: Option<ContentRef>,
    resolution: AccessHandlerResolution<S, S::Output, S::Failure, C>,
    objects: Vec<mfm_journal::single_trust::ImmutableObject>,
) -> Result<AccessConclusion<S>> {
    if !assembly.program().belongs_to_catalog(catalog) {
        return Err(RuntimeError::Identity);
    }
    let Some(mfm_program::Declaration::State(state)) =
        assembly.program().document().declaration(&occurrence)
    else {
        return Err(RuntimeError::Identity);
    };
    if state.execution().is_pure()
        || state.output_contract_ref() != &output_contract_ref
        || state.failure_contract_ref() != failure_contract_ref.as_ref()
    {
        return Err(RuntimeError::Identity);
    }
    if evidence_contract_ref != nominal_contract_ref::<C::Evidence>()? {
        return Err(RuntimeError::Value);
    }
    let (
        resolution_assembly_brand,
        input,
        recorded_call_id,
        intent,
        evidence,
        outcome,
        classification,
        recorded_preparation,
        fact_continuation,
    ) = resolution.into_parts();
    if !Arc::ptr_eq(&resolution_assembly_brand, &assembly.brand)
        || input.contract_ref() != state.input_contract_ref()
        || recorded_call_id != call_id
        || recorded_preparation != preparation
        || classification.is_some()
    {
        return Err(if classification.is_some() {
            RuntimeError::Unresolved
        } else {
            RuntimeError::Identity
        });
    }
    let Some(evidence) = evidence else {
        return Err(RuntimeError::Unresolved);
    };
    let Some(outcome) = outcome else {
        return Err(RuntimeError::Unresolved);
    };
    if !input.belongs_to_catalog(catalog) {
        return Err(RuntimeError::Identity);
    }
    if preparation.run_id() != run_id {
        return Err(RuntimeError::Identity);
    }
    let retained = store.load(run_id).map_err(|_| RuntimeError::Conclusion)?;
    let Some((selected, selected_ref)) = retained.selected_preparation(&occurrence) else {
        return Err(RuntimeError::Identity);
    };
    if selected.binding().capability_contract_ref() != state.execution().capability_contract_ref()
        || selected.execution_binding_ref()
            != state
                .execution_binding_ref()
                .ok_or(RuntimeError::Identity)?
    {
        return Err(RuntimeError::Identity);
    }
    let intent_ref = qualified_value_ref(&intent)?;
    if selected_ref != preparation
        || selected.input().value_ref() != input.value_ref()
        || selected.input().contract_ref() != input.contract_ref()
        || selected.intent().value_ref() != &intent_ref
        || selected.fact_request() != fact_continuation.as_ref().map(FactContinuation::request)
        || selected.fact_selection() != fact_continuation.as_ref().map(FactContinuation::selection)
    {
        return Err(RuntimeError::Identity);
    }
    if fact_continuation.as_ref().is_some_and(|continuation| {
        continuation.scope() != store.scope()
            || continuation.epoch() != store.epoch()
            || continuation.tenant() != store.tenant()
            || continuation.run_id() != run_id
            || continuation.preparation() != &preparation
    }) {
        return Err(RuntimeError::Identity);
    }
    let qualified_evidence = catalog
        .qualify(evidence_contract_ref.clone(), evidence)
        .map_err(|_| RuntimeError::Value)?;
    let evidence_ref = mfm_journal::single_trust::ValueRef::new(
        qualified_evidence.contract_ref().clone(),
        qualified_evidence.value_ref().clone(),
    );
    let mut objects = objects;
    let input_ref = mfm_journal::single_trust::ValueRef::new(
        input.contract_ref().clone(),
        input.value_ref().clone(),
    );
    objects.push(value_object(input.as_ref(), input_ref.value_ref())?);
    objects.push(value_object(&intent, &intent_ref)?);
    objects.push(value_object(
        qualified_evidence.as_ref(),
        evidence_ref.value_ref(),
    )?);
    let (outcome, success) = match outcome {
        ProposedStateOutcome::Success(output) => {
            let qualified = catalog
                .qualify(output_contract_ref.clone(), output)
                .map_err(|_| RuntimeError::Value)?;
            let value_ref = mfm_journal::single_trust::ValueRef::new(
                qualified.contract_ref().clone(),
                qualified.value_ref().clone(),
            );
            objects.push(value_object(qualified.as_ref(), value_ref.value_ref())?);
            (
                mfm_journal::single_trust::StateOutcome::Success(value_ref),
                Some(qualified),
            )
        }
        ProposedStateOutcome::Failure(failure) => {
            let failure_ref = failure_contract_ref.ok_or(RuntimeError::Value)?;
            let qualified = catalog
                .qualify(failure_ref, failure)
                .map_err(|_| RuntimeError::Value)?;
            let value_ref = mfm_journal::single_trust::ValueRef::new(
                qualified.contract_ref().clone(),
                qualified.value_ref().clone(),
            );
            objects.push(value_object(qualified.as_ref(), value_ref.value_ref())?);
            (
                mfm_journal::single_trust::StateOutcome::Failure(value_ref),
                None,
            )
        }
    };
    let owner = store
        .prepare_conclusion(
            run_id,
            assembly.program().document(),
            expected_sequence,
            append_request_id,
            mfm_journal::single_trust::StateConcluded::Access {
                occurrence,
                preparation,
                evidence: evidence_ref,
                outcome,
                fact_selection: fact_continuation
                    .as_ref()
                    .map(FactContinuation::selection)
                    .cloned(),
                fact_publication: None,
            },
            objects,
            mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
        )
        .map_err(|_| RuntimeError::Conclusion)?;
    match success {
        Some(successor) => Ok(AccessConclusion::Success(PendingConclusion::new(
            assembly, owner, successor,
        )?)),
        None => Ok(AccessConclusion::Failure(PendingFailure::new(
            assembly, owner,
        )?)),
    }
}

/// An inert qualified adapter.  Its provider operation is unreachable without a committed call.
pub struct QualifiedAdapter<S: State, C: AccessCapabilityContract> {
    binding_ref: ContentRef,
    state_implementation_ref: Option<ContentRef>,
    capability_contract_ref: Option<ContentRef>,
    adapter_implementation_ref: Option<ContentRef>,
    assembly_brand: Arc<RuntimeAssemblyBrand>,
    invoke: Arc<AccessExecutor<S, C>>,
}

impl<S: State, C: AccessCapabilityContract> QualifiedAdapter<S, C> {
    fn from_arc(
        assembly: &RuntimeAssembly,
        binding_ref: ContentRef,
        state_implementation_ref: Option<ContentRef>,
        capability_contract_ref: Option<ContentRef>,
        adapter_implementation_ref: Option<ContentRef>,
        invoke: Arc<AccessExecutor<S, C>>,
    ) -> Self {
        Self {
            binding_ref,
            state_implementation_ref,
            capability_contract_ref,
            adapter_implementation_ref,
            assembly_brand: Arc::clone(&assembly.brand),
            invoke,
        }
    }

    /// Returns the exact binding identity.
    pub const fn binding_ref(&self) -> &ContentRef {
        &self.binding_ref
    }

    /// Enters the provider-owned path with the one consuming committed call.
    pub fn enter(&self, call: CommittedCall<S, C>) -> AccessResolutionFuture<S, C> {
        if !Arc::ptr_eq(&self.assembly_brand, &call.assembly_brand)
            || call.execution_binding_ref() != &self.binding_ref
            || self
                .state_implementation_ref
                .as_ref()
                .is_some_and(|value| call.binding().state_implementation_ref() != value)
            || self
                .capability_contract_ref
                .as_ref()
                .is_some_and(|value| call.binding().capability_contract_ref() != Some(value))
            || self
                .adapter_implementation_ref
                .as_ref()
                .is_some_and(|value| call.binding().adapter_implementation_ref() != Some(value))
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
    inner: AccessResolutionFuture<S, C>,
}

impl<S: State, C: AccessCapabilityContract> Unpin for PanicContainedFuture<S, C> {}

impl<S: State, C: AccessCapabilityContract> Future for PanicContainedFuture<S, C> {
    type Output = Result<AccessHandlerResolution<S, S::Output, S::Failure, C>>;

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

struct RegisteredState {
    state_implementation_ref: ContentRef,
    state_type: TypeId,
    access: bool,
    capability_contract_ref: Option<ContentRef>,
    capability_type: Option<TypeId>,
    binding_ref: Option<ContentRef>,
    adapter_implementation_ref: Option<ContentRef>,
    implementation: Box<dyn Any + Send + Sync>,
    adapter: Option<Box<dyn Any + Send + Sync>>,
}

struct ProcessRegistry {
    states: Vec<RegisteredState>,
}

/// Builder for one immutable Program/Runtime association.
pub struct RuntimeAssemblyBuilder {
    catalog: ProgramCatalog,
    program: Program,
    registrations: Vec<RegisteredState>,
    brand: Arc<RuntimeAssemblyBrand>,
}

impl RuntimeAssemblyBuilder {
    /// Starts an assembly builder for one exact catalog and Program.
    pub fn new(catalog: ProgramCatalog, program: Program) -> Result<Self> {
        if !program.belongs_to_catalog(&catalog) {
            return Err(RuntimeError::Identity);
        }
        Ok(Self {
            catalog,
            program,
            registrations: Vec::new(),
            brand: Arc::new(RuntimeAssemblyBrand),
        })
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

    /// Registers one deterministic Pure State implementation.
    pub fn register_pure<S: State>(
        &mut self,
        state_implementation_ref: ContentRef,
        implementation: PureImplementation<S>,
    ) -> Result<()> {
        self.ensure_unique(&state_implementation_ref)?;
        self.registrations.push(RegisteredState {
            state_implementation_ref,
            state_type: TypeId::of::<S>(),
            access: false,
            capability_contract_ref: None,
            capability_type: None,
            binding_ref: None,
            adapter_implementation_ref: None,
            implementation: Box::new(implementation),
            adapter: None,
        });
        Ok(())
    }

    /// Registers one Access State implementation and its exact adapter callback.
    #[allow(clippy::too_many_arguments)]
    pub fn register_access<S: State, C: AccessCapabilityContract, F>(
        &mut self,
        state_implementation_ref: ContentRef,
        capability_contract_ref: ContentRef,
        execution_binding_ref: ContentRef,
        adapter_implementation_ref: ContentRef,
        implementation: AccessImplementation<S, C>,
        invoke: F,
    ) -> Result<()>
    where
        F: Fn(CommittedCall<S, C>) -> AccessResolutionFuture<S, C> + Send + Sync + 'static,
    {
        C::validate().map_err(|_| RuntimeError::Mode)?;
        if capability_contract_ref != capability_content_ref::<C>()? {
            return Err(RuntimeError::Identity);
        }
        self.ensure_unique(&state_implementation_ref)?;
        let adapter: Arc<AccessExecutor<S, C>> = Arc::new(invoke);
        self.registrations.push(RegisteredState {
            state_implementation_ref,
            state_type: TypeId::of::<S>(),
            access: true,
            capability_contract_ref: Some(capability_contract_ref),
            capability_type: Some(TypeId::of::<C>()),
            binding_ref: Some(execution_binding_ref),
            adapter_implementation_ref: Some(adapter_implementation_ref),
            implementation: Box::new(implementation),
            adapter: Some(Box::new(adapter)),
        });
        Ok(())
    }

    /// Finishes the immutable assembly after checking every Program registration.
    pub fn finish(self) -> Result<RuntimeAssembly> {
        let declarations = self
            .program
            .document()
            .declarations()
            .iter()
            .filter_map(|declaration| match declaration {
                mfm_program::Declaration::State(state) => Some(state),
                mfm_program::Declaration::Match(_) => None,
            })
            .collect::<Vec<_>>();
        if self.registrations.len() != declarations.len() {
            return Err(RuntimeError::Identity);
        }
        let mut registrations = self.registrations;
        let mut states = Vec::with_capacity(declarations.len());
        for state in declarations {
            let index = registrations
                .iter()
                .position(|registered| {
                    registered.state_implementation_ref == *state.state_implementation_ref()
                })
                .ok_or(RuntimeError::Identity)?;
            let registered = registrations.swap_remove(index);
            match state.execution() {
                mfm_program::ExecutionMode::Pure => {
                    if registered.access
                        || registered.binding_ref.is_some()
                        || registered.capability_contract_ref.is_some()
                        || registered.adapter.is_some()
                    {
                        return Err(RuntimeError::Mode);
                    }
                }
                mfm_program::ExecutionMode::Read {
                    capability_contract_ref,
                    ..
                }
                | mfm_program::ExecutionMode::Effect {
                    capability_contract_ref,
                    ..
                } => {
                    if !registered.access
                        || registered.binding_ref.as_ref() != state.execution_binding_ref()
                        || registered.capability_contract_ref.as_ref()
                            != Some(capability_contract_ref)
                        || registered.adapter.is_none()
                    {
                        return Err(RuntimeError::Mode);
                    }
                }
            }
            states.push(registered);
        }
        if !registrations.is_empty() {
            return Err(RuntimeError::Identity);
        }
        Ok(RuntimeAssembly {
            catalog: self.catalog,
            program: self.program,
            registry: ProcessRegistry { states },
            brand: self.brand,
        })
    }
}

/// Immutable process assembly brand shared by callback-free consumers.
pub struct RuntimeAssembly {
    catalog: ProgramCatalog,
    program: Program,
    registry: ProcessRegistry,
    brand: Arc<RuntimeAssemblyBrand>,
}

impl RuntimeAssembly {
    /// Starts an immutable Runtime assembly for one exact catalog and Program.
    pub fn new(catalog: ProgramCatalog, program: Program) -> Result<Self> {
        RuntimeAssemblyBuilder::new(catalog, program)?.finish()
    }

    /// Returns the exact callback-free Program catalog.
    pub const fn catalog(&self) -> &ProgramCatalog {
        &self.catalog
    }

    /// Returns the immutable callback-free Program.
    pub const fn program(&self) -> &Program {
        &self.program
    }

    /// Returns the exact Program content identity.
    pub const fn program_ref(&self) -> &ContentRef {
        self.program.program_ref().content_ref()
    }

    /// Returns whether another assembly is the same process-local authority.
    pub fn same_assembly(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.brand, &other.brand)
    }

    /// Returns one registered Pure implementation under its exact implementation identity.
    pub fn pure_implementation<S: State>(
        &self,
        state_implementation_ref: &ContentRef,
    ) -> Result<&PureImplementation<S>> {
        self.registry
            .states
            .iter()
            .find(|registered| {
                &registered.state_implementation_ref == state_implementation_ref
                    && registered.state_type == TypeId::of::<S>()
                    && registered.binding_ref.is_none()
            })
            .and_then(|registered| registered.implementation.downcast_ref())
            .ok_or(RuntimeError::Identity)
    }

    /// Returns one registered Access implementation under its exact State/capability identity.
    pub fn access_implementation<S: State, C: AccessCapabilityContract>(
        &self,
        state_implementation_ref: &ContentRef,
        capability_contract_ref: &ContentRef,
        execution_binding_ref: &ContentRef,
    ) -> Result<&AccessImplementation<S, C>> {
        self.registry
            .states
            .iter()
            .find(|registered| {
                &registered.state_implementation_ref == state_implementation_ref
                    && registered.state_type == TypeId::of::<S>()
                    && registered.capability_type == Some(TypeId::of::<C>())
                    && registered.capability_contract_ref.as_ref() == Some(capability_contract_ref)
                    && registered.binding_ref.as_ref() == Some(execution_binding_ref)
            })
            .and_then(|registered| registered.implementation.downcast_ref())
            .ok_or(RuntimeError::Identity)
    }

    /// Reifies one inert adapter from the exact registered Access binding.
    pub fn qualified_adapter<S: State, C: AccessCapabilityContract>(
        &self,
        state_implementation_ref: &ContentRef,
        capability_contract_ref: &ContentRef,
        execution_binding_ref: &ContentRef,
    ) -> Result<QualifiedAdapter<S, C>> {
        let registered = self
            .registry
            .states
            .iter()
            .find(|registered| {
                &registered.state_implementation_ref == state_implementation_ref
                    && registered.state_type == TypeId::of::<S>()
                    && registered.capability_type == Some(TypeId::of::<C>())
                    && registered.capability_contract_ref.as_ref() == Some(capability_contract_ref)
                    && registered.binding_ref.as_ref() == Some(execution_binding_ref)
            })
            .ok_or(RuntimeError::Identity)?;
        let invoke = registered
            .adapter
            .as_ref()
            .and_then(|adapter| adapter.downcast_ref::<Arc<AccessExecutor<S, C>>>())
            .ok_or(RuntimeError::Identity)?;
        Ok(QualifiedAdapter::from_arc(
            self,
            execution_binding_ref.clone(),
            Some(state_implementation_ref.clone()),
            Some(capability_contract_ref.clone()),
            registered.adapter_implementation_ref.clone(),
            Arc::clone(invoke),
        ))
    }
}

/// One reusable typed session retaining the latest cumulative value without cloning it.
pub struct RunSession<T: MfmValue> {
    assembly_brand: Arc<RuntimeAssemblyBrand>,
    value: QualifiedTypedValue<T>,
}

impl<T: MfmValue> RunSession<T> {
    /// Starts a session with one catalog-qualified typed value.
    pub fn new(assembly: &RuntimeAssembly, value: QualifiedTypedValue<T>) -> Result<Self> {
        if !value.belongs_to_catalog(assembly.catalog()) {
            return Err(RuntimeError::Identity);
        }
        Ok(Self {
            assembly_brand: Arc::clone(&assembly.brand),
            value,
        })
    }

    /// Borrows the exact latest context.
    pub const fn value(&self) -> &QualifiedTypedValue<T> {
        &self.value
    }

    /// Returns whether this session belongs to the exact immutable assembly.
    pub fn belongs_to_assembly(&self, assembly: &RuntimeAssembly) -> bool {
        Arc::ptr_eq(&self.assembly_brand, &assembly.brand)
    }

    /// Consumes the session and returns its sole typed owner.
    pub fn into_value(self) -> QualifiedTypedValue<T> {
        self.value
    }
}

/// Builds one typed successor without a serialize/decode handoff.
pub fn qualify_success<T: MfmValue>(
    assembly: &RuntimeAssembly,
    contract_ref: ContentRef,
    outcome: ProposedStateOutcome<T, impl MfmValue>,
) -> Result<RunSession<T>> {
    match outcome {
        ProposedStateOutcome::Success(value) => assembly
            .catalog()
            .qualify(contract_ref, value)
            .map_err(|_| RuntimeError::Value)
            .and_then(|value| RunSession::new(assembly, value)),
        ProposedStateOutcome::Failure(_) => Err(RuntimeError::Value),
    }
}

fn value_object<T: MfmValue>(value: &T, value_ref: &ContentRef) -> Result<ImmutableObject> {
    let canonical = canonical_value(value).map_err(|_| RuntimeError::Value)?;
    ImmutableObject::new(
        StableId::new("mfm.value").map_err(|_| RuntimeError::Value)?,
        value_ref.clone(),
        canonical.as_str().to_owned(),
    )
    .map_err(|_| RuntimeError::Value)
}

fn qualified_value_ref<T: MfmValue>(value: &T) -> Result<ContentRef> {
    let schema = T::schema_id().map_err(|_| RuntimeError::Value)?;
    let canonical = canonical_value(value).map_err(|_| RuntimeError::Value)?;
    ContentRef::new(
        schema,
        mfm_ids::ContentDigest::from_digest(
            mfm_ids::DigestAlgorithm::Sha256V1,
            canonical.digest_bytes(),
        ),
    )
    .map_err(|_| RuntimeError::Value)
}

fn nominal_contract_ref<T: MfmValue>() -> Result<ContentRef> {
    ContentRef::new(
        T::schema_id().map_err(|_| RuntimeError::Value)?,
        raw_content_digest(b"mfm.contract.v1"),
    )
    .map_err(|_| RuntimeError::Value)
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

fn capability_content_ref<C: AccessCapabilityContract>() -> Result<ContentRef> {
    let contract_id = C::contract_id().map_err(|_| RuntimeError::Mode)?;
    let schema = SchemaId::new(
        "mfm.capability-contract",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )
    .map_err(|_| RuntimeError::Identity)?;
    ContentRef::new(schema, raw_content_digest(contract_id.as_str().as_bytes()))
        .map_err(|_| RuntimeError::Identity)
}

/// Returns whether one preparation result is the exact direct-new branch.
pub const fn is_direct_new(append: &PreparationAppend) -> bool {
    matches!(
        append.disposition(),
        AppendDisposition::NewlyCommitted { .. }
    )
}

/// Keeps Store's result type name available to adapter integration without exposing a callback.
pub type StoreAppendResult = StoreResult<AppendDisposition>;

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_program_derive::MfmValue as DeriveMfmValue;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize, DeriveMfmValue)]
    #[serde(deny_unknown_fields)]
    struct Context {
        value: u64,
    }

    struct PureState;
    impl State for PureState {
        type Input = Context;
        type Output = Context;
        type Failure = Context;

        fn state_id() -> Result<StableId> {
            StableId::new("mfm.test.pure-state").map_err(|_| RuntimeError::Identity)
        }
    }

    struct TestRead;

    impl AccessCapabilityContract for TestRead {
        type Mode = ReadMode;
        type Intent = Context;
        type Evidence = Context;
        type Facts = mfm_capabilities::NoPriorFacts;

        fn contract_id() -> mfm_capabilities::Result<StableId> {
            StableId::new("mfm.test.read-capability")
                .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
        }

        fn total_attempt_bound() -> std::num::NonZeroU16 {
            std::num::NonZeroU16::new(1).expect("nonzero")
        }

        fn bind_evidence(
            _intent: &Self::Intent,
            _evidence: &Self::Evidence,
        ) -> mfm_capabilities::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn session_retains_non_clone_context() {
        assert_eq!(
            PureState::state_id().expect("state").as_str(),
            "mfm.test.pure-state"
        );
        let builder = ProgramCatalog::builder();
        let reference = || {
            let schema = Context::schema_id().expect("schema");
            ContentRef::new(
                schema,
                mfm_ids::ContentDigest::from_digest(
                    mfm_ids::DigestAlgorithm::Sha256V1,
                    mfm_ids::DigestBytes::from_array([1; 32]),
                ),
            )
            .expect("reference")
        };
        let doc = mfm_program::single_trust::ProgramDocument::new(
            StableId::new("mfm.test.entry").expect("entry"),
            reference(),
            reference(),
            Vec::new(),
        )
        .expect("document");
        let (catalog, program) = builder.finish(doc).expect("catalog");
        let assembly = RuntimeAssembly::new(catalog.clone(), program).expect("assembly");
        let value = catalog
            .qualify(reference(), Context { value: 7 })
            .expect("value");
        let session = RunSession::new(&assembly, value).expect("session");
        assert_eq!(session.value().as_ref().value, 7);
        let value = session.into_value().into_value();
        assert_eq!(value.value, 7);
    }

    #[test]
    fn assembly_requires_and_retrieves_typed_pure_registration() {
        let reference = || {
            let schema = Context::schema_id().expect("schema");
            ContentRef::new(
                schema,
                mfm_ids::ContentDigest::from_digest(
                    mfm_ids::DigestAlgorithm::Sha256V1,
                    mfm_ids::DigestBytes::from_array([3; 32]),
                ),
            )
            .expect("reference")
        };
        let implementation_ref = ContentRef::new(
            Context::schema_id().expect("schema"),
            mfm_ids::ContentDigest::from_digest(
                mfm_ids::DigestAlgorithm::Sha256V1,
                mfm_ids::DigestBytes::from_array([4; 32]),
            ),
        )
        .expect("implementation ref");
        let document = mfm_program::single_trust::ProgramDocument::new(
            StableId::new("mfm.test.registered-entry").expect("entry"),
            reference(),
            reference(),
            vec![mfm_program::Declaration::State(Box::new(
                mfm_program::single_trust::StateDeclaration::new(
                    SequentialControlAddress::new(0, Vec::new()).expect("address"),
                    implementation_ref.clone(),
                    reference(),
                    reference(),
                    None,
                    mfm_program::single_trust::ExecutionMode::Pure,
                    true,
                )
                .expect("state"),
            ))],
        )
        .expect("document");
        let (catalog, program) = ProgramCatalog::builder().finish(document).expect("program");
        let builder = RuntimeAssemblyBuilder::new(catalog.clone(), program).expect("builder");
        assert!(builder.finish().is_err());

        let document = mfm_program::single_trust::ProgramDocument::new(
            StableId::new("mfm.test.registered-entry").expect("entry"),
            reference(),
            reference(),
            vec![mfm_program::Declaration::State(Box::new(
                mfm_program::single_trust::StateDeclaration::new(
                    SequentialControlAddress::new(0, Vec::new()).expect("address"),
                    implementation_ref.clone(),
                    reference(),
                    reference(),
                    None,
                    mfm_program::single_trust::ExecutionMode::Pure,
                    true,
                )
                .expect("state"),
            ))],
        )
        .expect("document");
        let (catalog, program) = ProgramCatalog::builder().finish(document).expect("program");
        let mut builder = RuntimeAssemblyBuilder::new(catalog, program).expect("builder");
        builder
            .register_pure::<PureState>(
                implementation_ref.clone(),
                PureImplementation::<PureState>::new(|input| {
                    ProposedStateOutcome::Success(Context {
                        value: input.value + 1,
                    })
                }),
            )
            .expect("registration");
        let assembly = builder.finish().expect("assembly");
        let implementation = assembly
            .pure_implementation::<PureState>(&implementation_ref)
            .expect("implementation");
        assert_eq!(
            implementation.evaluate(&Context { value: 4 }),
            ProposedStateOutcome::Success(Context { value: 5 })
        );
    }

    #[test]
    fn pure_evaluation_panics_are_contained_before_conclusion() {
        let implementation = PureImplementation::<PureState>::new(|_| {
            panic!("test pure panic");
        });
        assert!(matches!(
            implementation.evaluate_contained(&Context { value: 4 }),
            Err(RuntimeError::Unresolved)
        ));
    }

    #[test]
    fn access_preparation_panics_are_contained_before_commit() {
        let reference = || {
            let schema = Context::schema_id().expect("schema");
            ContentRef::new(
                schema,
                mfm_ids::ContentDigest::from_digest(
                    mfm_ids::DigestAlgorithm::Sha256V1,
                    mfm_ids::DigestBytes::from_array([5; 32]),
                ),
            )
            .expect("reference")
        };
        let implementation_ref = reference();
        let capability_ref = capability_content_ref::<TestRead>().expect("capability");
        let adapter_ref = reference();
        let binding = BindingDescriptor::new(
            implementation_ref.clone(),
            Some(capability_ref.clone()),
            Some(adapter_ref.clone()),
            reference(),
            None,
            None,
        )
        .expect("binding");
        let binding_ref = binding.content_ref().expect("binding ref");
        let occurrence = SequentialControlAddress::new(0, Vec::new()).expect("occurrence");
        let document = mfm_program::ProgramDocument::new(
            StableId::new("mfm.test.preparation-panic").expect("entry"),
            reference(),
            reference(),
            vec![mfm_program::Declaration::State(Box::new(
                mfm_program::StateDeclaration::new(
                    occurrence.clone(),
                    implementation_ref.clone(),
                    reference(),
                    reference(),
                    None,
                    mfm_program::ExecutionMode::Read {
                        capability_contract_ref: capability_ref.clone(),
                        total_attempt_bound: 1,
                        fact_selection_required: false,
                    },
                    true,
                )
                .expect("state")
                .with_execution_binding(binding_ref.clone())
                .expect("binding"),
            ))],
        )
        .expect("document");
        let input_contract = match document.declaration(&occurrence) {
            Some(mfm_program::Declaration::State(state)) => state.input_contract_ref().clone(),
            _ => panic!("state declaration missing"),
        };
        let (catalog, program) = ProgramCatalog::builder().finish(document).expect("program");
        let mut builder = RuntimeAssemblyBuilder::new(catalog.clone(), program).expect("builder");
        builder
            .register_access::<PureState, TestRead, _>(
                implementation_ref,
                capability_ref,
                binding_ref.clone(),
                adapter_ref,
                AccessImplementation::new(
                    |_| panic!("test preparation panic"),
                    |_call| Box::pin(async { Err(RuntimeError::Unresolved) }),
                ),
                |_call| Box::pin(async { Err(RuntimeError::Unresolved) }),
            )
            .expect("registration");
        let assembly = builder.finish().expect("assembly");
        let input = catalog
            .qualify(input_contract, Context { value: 4 })
            .expect("input");
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run");
        assert!(matches!(
            PreparedExecution::<PureState, TestRead>::new(
                &assembly,
                run_id,
                occurrence,
                input,
                binding,
                binding_ref,
            ),
            Err(RuntimeError::Preparation)
        ));
    }

    #[test]
    fn adapter_future_panics_are_contained_as_unresolved() {
        let inner: AccessResolutionFuture<PureState, TestRead> = Box::pin(async {
            panic!("test adapter panic");
        });
        let mut future = PanicContainedFuture { inner };
        let waker = std::task::Waker::noop();
        let mut context = std::task::Context::from_waker(waker);
        let result = Future::poll(Pin::new(&mut future), &mut context);
        assert!(matches!(result, Poll::Ready(Err(RuntimeError::Unresolved))));
    }

    #[test]
    fn direct_new_call_correlation_uses_the_checked_stable_id_grammar() {
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run");
        let preparation = PreparationRef::new(run_id.clone(), 2, 1);
        let call_id = mint_call_id(&run_id, &preparation).expect("call id");
        assert_eq!(
            call_id.as_str(),
            "call/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/2/1"
        );
    }
}
