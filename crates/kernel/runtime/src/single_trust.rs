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
    AccessCapabilityContract, AccessMode, EffectMode, ProposedStateOutcome, ReadMode,
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
use mfm_store::single_trust::{AppendDisposition, FactContinuation, ReducedRunState};
use mfm_store::QualifiedRun;
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

/// A typed failure contract with one static integrity-blocked value.
///
/// The value is selected by the domain's closed failure contract and receives no input, evidence,
/// or Runtime authority. It is therefore safe for the capability-certified integrity path to use
/// without invoking a State callback or accepting a caller-supplied failure.
pub trait FailureValue: MfmValue {
    /// Returns the contract-fixed failure for an accepted integrity block.
    fn integrity_blocked() -> Self;
}

/// A callback-free typed State contract.
pub trait State: Send + Sync + 'static {
    /// Complete cumulative input consumed by this State.
    type Input: MfmValue;
    /// Complete successor context or terminal public result.
    type Output: MfmValue;
    /// Explicit fail-fast domain value.
    type Failure: FailureValue;

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

impl RuntimePreparationMode for EffectMode {
    fn journal_mode(_total_attempt_bound: std::num::NonZeroU16) -> PreparationMode {
        PreparationMode::Effect
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

    /// Consumes integrity evidence into the exact declared typed failure route.
    pub fn conclude_blocked(self) -> AccessHandlerResolution<S, S::Output, S::Failure, C> {
        AccessHandlerResolution {
            assembly_brand: self.assembly_brand,
            input: self.input,
            call_id: self.call_id,
            intent: self.intent,
            evidence: Some(self.evidence),
            outcome: Some(ProposedStateOutcome::Failure {
                failure: S::Failure::integrity_blocked(),
            }),
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
type AccessStateExecutor<S, C> =
    dyn Fn(CommittedCall<S, C>) -> AccessResolutionFuture<S, C> + Send + Sync;
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
    execute: Arc<AccessStateExecutor<S, C>>,
}

impl<S: State, C: AccessCapabilityContract> Clone for AccessImplementation<S, C> {
    fn clone(&self) -> Self {
        Self {
            prepare: Arc::clone(&self.prepare),
            execute: Arc::clone(&self.execute),
        }
    }
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
    mode: PreparationMode,
    fact_request: Option<(mfm_journal::single_trust::ValueRef, ImmutableObject)>,
    _mode: PhantomData<C::Mode>,
}

/// Result of consuming a preparation owner through an opened semantic Store.
#[allow(clippy::large_enum_variant)]
pub enum OpenedPreparationCommit<S: State, C: AccessCapabilityContract> {
    /// The preparation crossed Store direct-new and the call is now eligible for adapter entry.
    Direct {
        /// The inert committed call.
        call: CommittedCall<S, C>,
        /// The qualified prefix including the durable preparation.
        run: QualifiedRun,
        /// The retained reduction advanced to the selected preparation.
        reduced: ReducedRunState,
    },
    /// Store returned a non-new disposition; the exact inert owner remains available for
    /// acknowledgement resolution or semantic rebind.
    Retained {
        /// The original preparation owner, including its typed input and intent.
        owner: PreparedExecution<S, C>,
        /// The known Store disposition that prevented direct call minting.
        disposition: AppendDisposition,
    },
    /// The append owner could not cross a known boundary; the exact inert owner remains retained.
    Rejected {
        /// The original preparation owner.
        owner: PreparedExecution<S, C>,
        /// Redacted failure classification.
        error: RuntimeError,
    },
}

impl<S: State, C: AccessCapabilityContract> PreparedExecution<S, C>
where
    C::Mode: RuntimePreparationMode,
{
    /// Creates a preparation by borrowing input while retaining the consuming typed value.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
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
            _mode: PhantomData,
        })
    }

    /// Borrows the canonical intent before Store entry.
    pub const fn intent(&self) -> &C::Intent {
        &self.intent
    }

    /// Consumes this owner through the branded asynchronous Store boundary.
    #[allow(clippy::too_many_arguments)]
    pub async fn commit_opened(
        self,
        assembly: &RuntimeAssembly,
        store: &mfm_store::OpenedStructuredStore,
        current: &QualifiedRun,
        reduced: &ReducedRunState,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        input_ref: mfm_journal::single_trust::ValueRef,
        intent_ref: mfm_journal::single_trust::ValueRef,
        maximum_conclusion_bytes: u64,
        preparation_ordinal: u16,
        replaces: Option<PreparationRef>,
    ) -> OpenedPreparationCommit<S, C> {
        if !Arc::ptr_eq(&self.assembly_brand, &assembly.brand) {
            return OpenedPreparationCommit::Rejected {
                owner: self,
                error: RuntimeError::Identity,
            };
        }
        if C::requires_prior_facts() != self.fact_request.is_some() {
            return OpenedPreparationCommit::Rejected {
                owner: self,
                error: RuntimeError::Preparation,
            };
        }
        let expected_intent_contract = match nominal_contract_ref::<C::Intent>() {
            Ok(value) => value,
            Err(error) => return OpenedPreparationCommit::Rejected { owner: self, error },
        };
        let expected_intent_value = match qualified_value_ref(&self.intent) {
            Ok(value) => value,
            Err(error) => return OpenedPreparationCommit::Rejected { owner: self, error },
        };
        if self.input.contract_ref() != input_ref.contract_ref()
            || self.input.value_ref() != input_ref.value_ref()
            || intent_ref.contract_ref() != &expected_intent_contract
            || intent_ref.value_ref() != &expected_intent_value
            || !intent_ref.is_schema_bound()
        {
            return OpenedPreparationCommit::Rejected {
                owner: self,
                error: RuntimeError::Value,
            };
        }
        let prepared = match StatePrepared::new(
            self.occurrence.clone(),
            preparation_ordinal,
            input_ref.clone(),
            intent_ref.clone(),
            self.fact_request
                .as_ref()
                .map(|(request, _)| request.clone()),
            None,
            self.mode,
            self.binding.clone(),
            self.execution_binding_ref.clone(),
            replaces,
            maximum_conclusion_bytes,
        ) {
            Ok(prepared) => prepared,
            Err(_) => {
                return OpenedPreparationCommit::Rejected {
                    owner: self,
                    error: RuntimeError::Preparation,
                }
            }
        };
        let mut objects = vec![
            match value_object(self.input.as_ref(), input_ref.value_ref()) {
                Ok(object) => object,
                Err(error) => return OpenedPreparationCommit::Rejected { owner: self, error },
            },
            match value_object(&self.intent, intent_ref.value_ref()) {
                Ok(object) => object,
                Err(error) => return OpenedPreparationCommit::Rejected { owner: self, error },
            },
        ];
        if let Some((_, object)) = &self.fact_request {
            objects.push(object.clone());
        }
        let append = match store
            .prepare_access_qualified_with_reduced(
                current,
                assembly.program().document(),
                reduced,
                expected_sequence,
                append_request_id,
                prepared,
                objects,
            )
            .await
        {
            Ok(append) => append,
            Err(_) => {
                return OpenedPreparationCommit::Rejected {
                    owner: self,
                    error: RuntimeError::Preparation,
                }
            }
        };
        if !matches!(
            append.disposition(),
            AppendDisposition::NewlyCommitted { .. }
        ) {
            return OpenedPreparationCommit::Retained {
                owner: self,
                disposition: append.disposition(),
            };
        }
        let preparation = match append.preparation().cloned() {
            Some(preparation) => preparation,
            None => {
                return OpenedPreparationCommit::Rejected {
                    owner: self,
                    error: RuntimeError::PreparationNotCommitted,
                }
            }
        };
        let call_id = match mint_call_id(&self.run_id, &preparation) {
            Ok(call_id) => call_id,
            Err(error) => return OpenedPreparationCommit::Rejected { owner: self, error },
        };
        let capability_contract_ref = match self.binding.capability_contract_ref() {
            Some(value) => value,
            None => {
                return OpenedPreparationCommit::Rejected {
                    owner: self,
                    error: RuntimeError::Identity,
                }
            }
        };
        let adapter = match assembly.qualified_adapter::<S, C>(
            self.binding.state_implementation_ref(),
            capability_contract_ref,
            &self.execution_binding_ref,
        ) {
            Ok(adapter) => Arc::new(adapter),
            Err(error) => return OpenedPreparationCommit::Rejected { owner: self, error },
        };
        let committed_run = match append.committed_frame() {
            Some(frame) => match store.qualify_appended(current, frame.clone()) {
                Ok(run) => run,
                Err(error) => {
                    return OpenedPreparationCommit::Rejected {
                        owner: self,
                        error: error.into(),
                    }
                }
            },
            None => current.clone(),
        };
        let reduced = match reduced.waiting_preparation(
            &committed_run,
            self.occurrence.clone(),
            preparation.clone(),
        ) {
            Ok(reduced) => reduced,
            Err(error) => {
                return OpenedPreparationCommit::Rejected {
                    owner: self,
                    error: error.into(),
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
                fact_continuation: append.into_fact_continuation(),
                adapter,
            },
            run: committed_run,
            reduced,
        }
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
    pub fn enter(&self, call: CommittedCall<S, C>) -> AccessIngressFuture<S, C> {
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
    pub(crate) access: bool,
    pub(crate) capability_contract_ref: Option<ContentRef>,
    pub(crate) capability_type: Option<TypeId>,
    pub(crate) binding_ref: Option<ContentRef>,
    pub(crate) adapter_implementation_ref: Option<ContentRef>,
    pub(crate) implementation: Box<dyn Any + Send + Sync>,
    pub(crate) adapter: Option<Box<dyn Any + Send + Sync>>,
    pub(crate) dynamic: Option<Arc<dyn crate::lifecycle::DynamicStateRegistration>>,
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
        let dynamic = crate::lifecycle::pure_registration(implementation.clone());
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
            dynamic: Some(dynamic),
        });
        Ok(())
    }

    /// Registers one Access State with its immutable content-addressed binding descriptor.
    #[allow(clippy::too_many_arguments)]
    pub fn register_access_with_binding<S: State, C: AccessCapabilityContract, F>(
        &mut self,
        state_implementation_ref: ContentRef,
        capability_contract_ref: ContentRef,
        execution_binding_ref: ContentRef,
        adapter_implementation_ref: ContentRef,
        binding: BindingDescriptor,
        implementation: AccessImplementation<S, C>,
        invoke: F,
    ) -> Result<()>
    where
        F: Fn(CommittedCall<S, C>) -> AccessIngressFuture<S, C> + Send + Sync + 'static,
        C::Mode: RuntimePreparationMode,
    {
        C::validate().map_err(|_| RuntimeError::Mode)?;
        if capability_contract_ref != capability_content_ref::<C>()?
            || binding.state_implementation_ref() != &state_implementation_ref
            || binding.capability_contract_ref() != Some(&capability_contract_ref)
            || binding.adapter_implementation_ref() != Some(&adapter_implementation_ref)
            || binding.content_ref().map_err(|_| RuntimeError::Identity)? != execution_binding_ref
        {
            return Err(RuntimeError::Identity);
        }
        self.ensure_unique(&state_implementation_ref)?;
        let adapter: Arc<AccessExecutor<S, C>> = Arc::new(invoke);
        let dynamic =
            crate::lifecycle::access_registration_with_binding(implementation.clone(), binding);
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
            dynamic: Some(dynamic),
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
            .and_then(|registered| registered.dynamic.as_ref())
            .cloned()
            .ok_or(RuntimeError::Identity)
    }

    pub(crate) fn reify_value(
        &self,
        witness: &Arc<crate::lifecycle::RuntimeWitness>,
        contract: &ContentRef,
        canonical_bytes: &[u8],
    ) -> Result<crate::lifecycle::ErasedValue> {
        for registered in &self.registry.states {
            if let Some(dynamic) = registered.dynamic.as_ref() {
                if let Some(value) =
                    dynamic.reify(&self.catalog, witness, contract, canonical_bytes)?
                {
                    return Ok(value);
                }
            }
        }
        Err(RuntimeError::Identity)
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

pub(crate) fn capability_content_ref<C: AccessCapabilityContract>() -> Result<ContentRef> {
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

    impl FailureValue for Context {
        fn integrity_blocked() -> Self {
            Self { value: 0 }
        }
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
        let (catalog, _program) = builder.finish(doc).expect("catalog");
        let value = catalog
            .qualify(reference(), Context { value: 7 })
            .expect("value");
        assert_eq!(value.as_ref().value, 7);
        let value = value.into_value();
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
                PureImplementation::<PureState>::new(|input| ProposedStateOutcome::Success {
                    output: Context {
                        value: input.value + 1,
                    },
                    facts: mfm_facts::FactProposalSet::empty(),
                }),
            )
            .expect("registration");
        let assembly = builder.finish().expect("assembly");
        let implementation = assembly
            .pure_implementation::<PureState>(&implementation_ref)
            .expect("implementation");
        assert_eq!(
            implementation.evaluate(&Context { value: 4 }),
            ProposedStateOutcome::Success {
                output: Context { value: 5 },
                facts: mfm_facts::FactProposalSet::empty(),
            }
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
            .register_access_with_binding::<PureState, TestRead, _>(
                implementation_ref,
                capability_ref,
                binding_ref.clone(),
                adapter_ref,
                binding.clone(),
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
        let inner: AccessIngressFuture<PureState, TestRead> = Box::pin(async {
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
