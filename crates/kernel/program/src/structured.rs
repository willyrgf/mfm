//! Typed authoring surface for declaration-ordered structured programs.
//!
//! The builders preserve declaration order, never execute callbacks, and
//! expose no store or live-access authority.  Certification remains
//! authoritative for hostile serialized input.

use std::any::TypeId;
use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::sync::Arc;

use mfm_capabilities::{
    EffectAdapterInvoker, EffectCapabilityContract, EffectRefreshMode, NoRefresh,
    ReadAdapterInvoker, ReadCapabilityContract, Refreshable, ResourceAuthorityContract,
    SignerContract,
};
use mfm_ids::{ContentRef, StableId};
use mfm_spec::structured::{
    access_fault_contract_ref, failure_handler_semantic_call_id, fan_out_join_contract_ref,
    policy_expansion_recipe_ref, policy_proceed_program_ref,
    prior_run_fact_selection_capability_contract, retained_value_contract_ref,
    structured_value_contract, structured_value_contract_ref, AuthoredBlock, AuthoredDeclaration,
    AuthoredFailureDirective, AuthoredFanOut, AuthoredFanOutLane, AuthoredMatch, AuthoredMatchArm,
    AuthoredOperationCall, AuthoredStateCall, AuthoredStructuredProgram, BlockTail,
    ClosedSumContract, ClosedSumPayload, ClosedSumVariant, FailureMapperRegistration, FailureScope,
    FailureScopeBinding, FragmentInputBinding, LaneOutcome, LexicalProducer, LexicalSlot,
    ProposedStateOutcome, ResultRole, SemanticCallPath, SemanticPathSegment, StructuralPath,
    StructuralPathSegment, StructuredCapabilityProtocolContract, StructuredEffectRefreshContract,
    StructuredExecutionKind, StructuredFailureContract, StructuredLiveComponentContract,
    StructuredSafeFailureDispositionContract, StructuredStateContract,
    StructuredStateExecutionContract,
};

/// Success-only proposal admitted by safe-failure settlement under
/// [`SafeFailureSuccessOnly`].
pub use mfm_spec::structured::ProposedSuccessOutcome;
use mfm_values::{
    component_object_evidence_contract_ref, MfmValue, RetainedValueContract, SchemaShape,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{ProgramError, Result};

/// Kernel-owned uninhabited state-failure type.
///
/// It deliberately has no `MfmValue` implementation, schema, codec, retained
/// slot, or producer.
///
/// ```compile_fail
/// use mfm_program::structured::{Never, PendingState, State};
/// use mfm_ids::StableId;
/// use mfm_program::structured::{Direct, Pure};
/// use mfm_program::structured::SafeFailureNotApplicable;
///
/// struct Infallible;
/// impl State for Infallible {
///     type Input = ();
///     type Output = ();
///     type Failure = Never;
///     type Request = ();
///     type Returned = ();
///     type SafeFailure = ();
///     type Execution = Pure;
///     type SafeFailureDisposition = SafeFailureNotApplicable;
///     type Capability = Direct;
///
///     fn semantic_state_id() -> mfm_program::Result<StableId> {
///         StableId::new("infallible").map_err(|error| error.into())
///     }
/// }
///
/// fn cannot_attach_a_handler(pending: PendingState<'_, Infallible, Never>) {
///     let _ = pending.or_default();
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Never {}

impl<'de> Deserialize<'de> for Never {
    fn deserialize<D>(_deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Err(serde::de::Error::custom(
            "the kernel Never type has no canonical value",
        ))
    }
}

mod private {
    use super::{
        Direct, Effect, FailureValue, FanOutAtLimit, FanOutOneRemaining, FanOutResults, MfmValue,
        Never, Pure, Read, RequiresCapability, RuntimeEffectCapability, RuntimeReadCapability,
        SafeFailureMayFail, SafeFailureNotApplicable, SafeFailureSuccessOnly, Sequential,
        StructuredValue,
    };

    pub trait FailureValueSealed {}

    impl FailureValueSealed for Never {}
    impl<T: MfmValue> FailureValueSealed for T {}

    pub trait StructuredValueSealed {}

    impl<T: MfmValue> StructuredValueSealed for T {}
    impl<Output, Failure> StructuredValueSealed for FanOutResults<Output, Failure>
    where
        Output: StructuredValue,
        Failure: FailureValue,
    {
    }

    pub trait ExecutionSealed {}

    impl ExecutionSealed for Pure {}
    impl<Capability: RuntimeReadCapability> ExecutionSealed for Read<Capability> {}
    impl<Capability: RuntimeEffectCapability> ExecutionSealed for Effect<Capability> {}

    pub trait SafeFailureDispositionSealed<Execution, Failure> {}

    impl<Failure: FailureValue> SafeFailureDispositionSealed<Pure, Failure>
        for SafeFailureNotApplicable
    {
    }

    impl<Capability, Failure> SafeFailureDispositionSealed<Read<Capability>, Failure>
        for SafeFailureSuccessOnly
    where
        Capability: RuntimeReadCapability,
        Failure: FailureValue,
    {
    }

    impl<Capability, Failure> SafeFailureDispositionSealed<Effect<Capability>, Failure>
        for SafeFailureSuccessOnly
    where
        Capability: RuntimeEffectCapability,
        Failure: FailureValue,
    {
    }

    impl<Capability, Failure> SafeFailureDispositionSealed<Read<Capability>, Failure>
        for SafeFailureMayFail
    where
        Capability: RuntimeReadCapability,
        Failure: MfmValue,
    {
    }

    impl<Capability, Failure> SafeFailureDispositionSealed<Effect<Capability>, Failure>
        for SafeFailureMayFail
    where
        Capability: RuntimeEffectCapability,
        Failure: MfmValue,
    {
    }

    pub trait CapabilitySealed {}

    pub trait RuntimeEffectRefreshBindingSealed<Mode> {}

    impl CapabilitySealed for Direct {}
    impl<Expansion> CapabilitySealed for RequiresCapability<Expansion> {}

    pub trait AuthoringPolicySealed {}

    impl AuthoringPolicySealed for Sequential {}
    impl AuthoringPolicySealed for FanOutOneRemaining {}
    impl AuthoringPolicySealed for FanOutAtLimit {}
}

/// Sealed failure classification for exact kernel [`Never`] or an inhabited
/// [`MfmValue`].
pub trait FailureValue:
    private::FailureValueSealed + Serialize + DeserializeOwned + Send + Sync + 'static
{
    /// Returns the one exact failure contract.
    fn failure_contract() -> Result<StructuredFailureContract>;
}

impl FailureValue for Never {
    fn failure_contract() -> Result<StructuredFailureContract> {
        Ok(StructuredFailureContract::never())
    }
}

impl<T> FailureValue for T
where
    T: MfmValue,
{
    fn failure_contract() -> Result<StructuredFailureContract> {
        StructuredFailureContract::typed(structured_value_contract::<T>()?).map_err(Into::into)
    }
}

/// Sealed relation deriving one exact state safe-failure disposition and the
/// exact proposal type admitted by safe-failure settlement.
pub trait SafeFailureDisposition<ExecutionKind, Failure>:
    private::SafeFailureDispositionSealed<ExecutionKind, Failure> + Send + Sync + 'static
where
    ExecutionKind: Execution,
    Failure: FailureValue,
{
    /// Exact safe-failure settlement proposal admitted by this disposition.
    ///
    /// Success-only dispositions expose [`ProposedSuccessOutcome`], which cannot
    /// express `Failure` or `InvalidEvidence`. May-fail dispositions expose
    /// [`ProposedStateOutcome`] (success or typed failure only). Pure states
    /// expose an uninhabited proposal because they never settle observations.
    type SafeFailureProposal<Output>: Serialize + Send + Sync + 'static
    where
        Output: Serialize + Send + Sync + 'static;

    /// Derives the canonical state-contract disposition.
    #[doc(hidden)]
    fn contract() -> StructuredSafeFailureDispositionContract;

    /// Lifts one disposition-admitted safe-failure proposal into store settlement.
    #[doc(hidden)]
    fn into_settlement<Output>(
        proposal: Self::SafeFailureProposal<Output>,
    ) -> StateSettlement<Output, Failure>
    where
        Output: Serialize + Send + Sync + 'static;
}

/// Marker for `Pure` states, which cannot receive access observations.
pub enum SafeFailureNotApplicable {}

impl<Failure> SafeFailureDisposition<Pure, Failure> for SafeFailureNotApplicable
where
    Failure: FailureValue,
{
    type SafeFailureProposal<Output>
        = Never
    where
        Output: Serialize + Send + Sync + 'static;

    fn contract() -> StructuredSafeFailureDispositionContract {
        StructuredSafeFailureDispositionContract::NotApplicable {}
    }

    fn into_settlement<Output>(
        proposal: Self::SafeFailureProposal<Output>,
    ) -> StateSettlement<Output, Failure>
    where
        Output: Serialize + Send + Sync + 'static,
    {
        match proposal {}
    }
}

/// Marker requiring every admitted safe failure to settle successfully.
///
/// The safe-failure callback returns [`ProposedSuccessOutcome`] only: the type
/// system rejects `Failure` and `InvalidEvidence` for every inhabited value.
pub enum SafeFailureSuccessOnly {}

impl<Capability, Failure> SafeFailureDisposition<Read<Capability>, Failure>
    for SafeFailureSuccessOnly
where
    Capability: RuntimeReadCapability,
    Failure: FailureValue,
{
    type SafeFailureProposal<Output>
        = ProposedSuccessOutcome<Output>
    where
        Output: Serialize + Send + Sync + 'static;

    fn contract() -> StructuredSafeFailureDispositionContract {
        StructuredSafeFailureDispositionContract::AllValidEvidenceSettlesSuccess {}
    }

    fn into_settlement<Output>(
        proposal: Self::SafeFailureProposal<Output>,
    ) -> StateSettlement<Output, Failure>
    where
        Output: Serialize + Send + Sync + 'static,
    {
        StateSettlement::Proposed(proposal.into_proposed_outcome())
    }
}

impl<Capability, Failure> SafeFailureDisposition<Effect<Capability>, Failure>
    for SafeFailureSuccessOnly
where
    Capability: RuntimeEffectCapability,
    Failure: FailureValue,
{
    type SafeFailureProposal<Output>
        = ProposedSuccessOutcome<Output>
    where
        Output: Serialize + Send + Sync + 'static;

    fn contract() -> StructuredSafeFailureDispositionContract {
        StructuredSafeFailureDispositionContract::AllValidEvidenceSettlesSuccess {}
    }

    fn into_settlement<Output>(
        proposal: Self::SafeFailureProposal<Output>,
    ) -> StateSettlement<Output, Failure>
    where
        Output: Serialize + Send + Sync + 'static,
    {
        StateSettlement::Proposed(proposal.into_proposed_outcome())
    }
}

/// Marker allowing admitted safe failures to settle to an inhabited typed failure.
///
/// The safe-failure callback returns [`ProposedStateOutcome`] only: success or
/// typed failure. `InvalidEvidence` remains reserved for returned-value settlement.
pub enum SafeFailureMayFail {}

impl<Capability, Failure> SafeFailureDisposition<Read<Capability>, Failure> for SafeFailureMayFail
where
    Capability: RuntimeReadCapability,
    Failure: MfmValue,
{
    type SafeFailureProposal<Output>
        = ProposedStateOutcome<Output, Failure>
    where
        Output: Serialize + Send + Sync + 'static;

    fn contract() -> StructuredSafeFailureDispositionContract {
        StructuredSafeFailureDispositionContract::MaySettleTypedFailure {}
    }

    fn into_settlement<Output>(
        proposal: Self::SafeFailureProposal<Output>,
    ) -> StateSettlement<Output, Failure>
    where
        Output: Serialize + Send + Sync + 'static,
    {
        StateSettlement::Proposed(proposal)
    }
}

impl<Capability, Failure> SafeFailureDisposition<Effect<Capability>, Failure> for SafeFailureMayFail
where
    Capability: RuntimeEffectCapability,
    Failure: MfmValue,
{
    type SafeFailureProposal<Output>
        = ProposedStateOutcome<Output, Failure>
    where
        Output: Serialize + Send + Sync + 'static;

    fn contract() -> StructuredSafeFailureDispositionContract {
        StructuredSafeFailureDispositionContract::MaySettleTypedFailure {}
    }

    fn into_settlement<Output>(
        proposal: Self::SafeFailureProposal<Output>,
    ) -> StateSettlement<Output, Failure>
    where
        Output: Serialize + Send + Sync + 'static,
    {
        StateSettlement::Proposed(proposal)
    }
}

/// Sealed typed lexical-value contract used by operation roots and structural
/// joins.
pub trait StructuredValue:
    private::StructuredValueSealed + Serialize + DeserializeOwned + Send + Sync + 'static
{
    /// Derives the exact nominal contract for this lexical value role.
    fn structured_contract_ref() -> Result<ContentRef>;
}

impl<T> StructuredValue for T
where
    T: MfmValue,
{
    fn structured_contract_ref() -> Result<ContentRef> {
        structured_value_contract_ref::<T>().map_err(Into::into)
    }
}

/// Sealed semantic execution classification for one state type.
pub trait Execution: private::ExecutionSealed + Send + Sync + 'static {
    /// Derives the exact local or live-capability execution contract.
    fn execution_contract() -> Result<StructuredStateExecutionContract>;

    /// Returns the exact typed access ABI for Read/Effect execution.
    #[doc(hidden)]
    fn access_type_ids() -> Option<(TypeId, TypeId, TypeId)>;
}

/// Deterministic local state execution with no semantic external IO.
pub struct Pure;

impl Execution for Pure {
    fn execution_contract() -> Result<StructuredStateExecutionContract> {
        Ok(StructuredStateExecutionContract::Pure)
    }

    fn access_type_ids() -> Option<(TypeId, TypeId, TypeId)> {
        None
    }
}

/// Type-derived runtime capability contract selected by a Read state.
pub trait RuntimeReadCapability: ReadCapabilityContract + Send + Sync + 'static {
    /// Returns the exact semantic Read capability contract.
    fn contract() -> Result<StructuredLiveComponentContract>;
}

/// Kernel-owned ordinary Read capability for complete prior-run fact selection.
pub enum PriorRunFactSelectionCapability {}

impl ReadCapabilityContract for PriorRunFactSelectionCapability {
    type Request = mfm_facts::FactSelectionRequest;
    type Returned = mfm_facts::FactSelectionReadResponse;
    type SafeFailure = mfm_facts::FactSelectionReadFailure;
}

impl RuntimeReadCapability for PriorRunFactSelectionCapability {
    fn contract() -> Result<StructuredLiveComponentContract> {
        prior_run_fact_selection_capability_contract().map_err(Into::into)
    }
}

/// Derives and checks the exact typed semantic contract of one Runtime Read capability.
pub fn runtime_read_capability_contract<Capability>() -> Result<StructuredLiveComponentContract>
where
    Capability: RuntimeReadCapability,
{
    let contract = Capability::contract()?;
    let expected = StructuredCapabilityProtocolContract::Read {
        request_contract_ref: structured_value_contract_ref::<Capability::Request>()?,
        returned_contract_ref: structured_value_contract_ref::<Capability::Returned>()?,
        safe_failure_contract_ref: structured_value_contract_ref::<Capability::SafeFailure>()?,
        access_fault_contract_ref: access_fault_contract_ref()?,
    };
    if contract.component_kind != mfm_spec::structured::StructuredComponentKind::Capability
        || contract.capability_protocol.as_ref() != Some(&expected)
    {
        return Err(ProgramError::Authoring(
            "runtime capability contract does not bind its exact typed protocol".to_owned(),
        ));
    }
    Ok(contract)
}

/// Type-derived runtime capability contract selected by an Effect state.
pub trait RuntimeEffectCapability: EffectCapabilityContract + Send + Sync + 'static {
    /// Sealed typed binding that derives exact refresh evidence and resource
    /// lineage contracts for this capability's refresh mode.
    type RefreshBinding: RuntimeEffectRefreshBinding<Self::Refresh>;

    /// Returns the exact semantic Effect capability contract.
    fn contract() -> Result<StructuredLiveComponentContract>;
}

/// Sealed derivation of one Effect refresh-mode contract.
pub trait RuntimeEffectRefreshBinding<Mode>:
    private::RuntimeEffectRefreshBindingSealed<Mode> + Send + Sync + 'static
where
    Mode: EffectRefreshMode,
{
    /// Derives the exact canonical refresh contract.
    fn contract() -> Result<StructuredEffectRefreshContract>;
}

/// Typed binding for an ordinary non-refreshable Effect.
pub enum NoRefreshBinding {}

impl private::RuntimeEffectRefreshBindingSealed<NoRefresh> for NoRefreshBinding {}

impl RuntimeEffectRefreshBinding<NoRefresh> for NoRefreshBinding {
    fn contract() -> Result<StructuredEffectRefreshContract> {
        Ok(StructuredEffectRefreshContract::NoRefresh {})
    }
}

/// Typed binding from refresh evidence to the exact stable resource lineage.
pub struct RefreshableBinding<Resource>(PhantomData<fn() -> Resource>);

impl<E, Resource> private::RuntimeEffectRefreshBindingSealed<Refreshable<E>>
    for RefreshableBinding<Resource>
where
    E: MfmValue,
    Resource: RuntimeResourceAuthority,
{
}

impl<E, Resource> RuntimeEffectRefreshBinding<Refreshable<E>> for RefreshableBinding<Resource>
where
    E: MfmValue,
    Resource: RuntimeResourceAuthority,
{
    fn contract() -> Result<StructuredEffectRefreshContract> {
        Ok(StructuredEffectRefreshContract::Refreshable {
            refresh_evidence_contract_ref: structured_value_contract_ref::<E>()?,
            resource_lineage_contract_ref: Box::new(Resource::contract()?.content_ref()?),
        })
    }
}

/// Derives and checks the exact typed semantic contract of one Runtime Effect capability.
pub fn runtime_effect_capability_contract<Capability>() -> Result<StructuredLiveComponentContract>
where
    Capability: RuntimeEffectCapability,
{
    let contract = Capability::contract()?;
    let expected = StructuredCapabilityProtocolContract::Effect {
        request_contract_ref: structured_value_contract_ref::<Capability::Request>()?,
        returned_contract_ref: structured_value_contract_ref::<Capability::Returned>()?,
        safe_failure_contract_ref: structured_value_contract_ref::<Capability::SafeFailure>()?,
        access_fault_contract_ref: access_fault_contract_ref()?,
        refresh_contract: Capability::RefreshBinding::contract()?,
    };
    if contract.component_kind != mfm_spec::structured::StructuredComponentKind::Capability
        || contract.capability_protocol.as_ref() != Some(&expected)
    {
        return Err(ProgramError::Authoring(
            "runtime Effect capability contract does not bind its exact typed protocol".to_owned(),
        ));
    }
    Ok(contract)
}

/// Type-derived Runtime Read adapter bound to one exact Read capability.
pub trait RuntimeReadAdapter<C>: ReadAdapterInvoker<C> + Send + Sync + 'static
where
    C: RuntimeReadCapability,
{
    /// Returns the exact semantic adapter contract.
    fn contract() -> Result<StructuredLiveComponentContract>;
}

/// Type-derived Runtime Effect adapter bound to one exact Effect capability.
pub trait RuntimeEffectAdapter<C>: EffectAdapterInvoker<C> + Send + Sync + 'static
where
    C: RuntimeEffectCapability,
{
    /// Returns the exact semantic adapter contract.
    fn contract() -> Result<StructuredLiveComponentContract>;
}

/// Type-derived Runtime signer contract retained by qualified assembly.
pub trait RuntimeSigner: SignerContract + Send + Sync + 'static {
    /// Returns the exact semantic signer contract.
    fn contract() -> Result<StructuredLiveComponentContract>;
}

/// Type-derived Runtime resource-authority contract retained by qualified assembly.
pub trait RuntimeResourceAuthority: ResourceAuthorityContract + Send + Sync + 'static {
    /// Returns the exact semantic resource-authority contract.
    fn contract() -> Result<StructuredLiveComponentContract>;
}

/// One typed external observation that does not intentionally mutate its target.
pub struct Read<Capability>(PhantomData<fn() -> Capability>);

impl<Capability> Execution for Read<Capability>
where
    Capability: RuntimeReadCapability,
{
    fn execution_contract() -> Result<StructuredStateExecutionContract> {
        Ok(StructuredStateExecutionContract::Read {
            capability_contract_ref: runtime_read_capability_contract::<Capability>()?
                .content_ref()?,
        })
    }

    fn access_type_ids() -> Option<(TypeId, TypeId, TypeId)> {
        Some((
            TypeId::of::<Capability::Request>(),
            TypeId::of::<Capability::Returned>(),
            TypeId::of::<Capability::SafeFailure>(),
        ))
    }
}

/// One typed operation that may mutate or consume an exclusive external capability.
pub struct Effect<Capability>(PhantomData<fn() -> Capability>);

impl<Capability> Execution for Effect<Capability>
where
    Capability: RuntimeEffectCapability,
{
    fn execution_contract() -> Result<StructuredStateExecutionContract> {
        Ok(StructuredStateExecutionContract::Effect {
            capability_contract_ref: runtime_effect_capability_contract::<Capability>()?
                .content_ref()?,
        })
    }

    fn access_type_ids() -> Option<(TypeId, TypeId, TypeId)> {
        Some((
            TypeId::of::<Capability::Request>(),
            TypeId::of::<Capability::Returned>(),
            TypeId::of::<Capability::SafeFailure>(),
        ))
    }
}

/// Pure declarative expansion selected by an abstract semantic capability state.
pub trait CapabilityExpansion: Send + Sync + 'static {
    /// Returns the exact callback-free authored expansion recipe.
    fn recipe() -> Result<AuthoredStructuredProgram>;
}

/// Sealed state capability classification.
pub trait Capability: private::CapabilitySealed + Send + Sync + 'static {
    /// Derives the optional exact capability requirement from declarative data.
    fn requirement_ref() -> Result<Option<ContentRef>>;
}

/// Directly implemented state with no semantic capability lowering requirement.
pub struct Direct;

impl Capability for Direct {
    fn requirement_ref() -> Result<Option<ContentRef>> {
        Ok(None)
    }
}

/// Abstract state lowered by the exact recipe owned by `Expansion`.
pub struct RequiresCapability<Expansion>(PhantomData<fn() -> Expansion>);

impl<Expansion> Capability for RequiresCapability<Expansion>
where
    Expansion: CapabilityExpansion,
{
    fn requirement_ref() -> Result<Option<ContentRef>> {
        let recipe_ref = Expansion::recipe()?.content_ref()?;
        Ok(Some(
            mfm_spec::structured::capability_expansion_requirement_ref(&recipe_ref)?,
        ))
    }
}

/// Metadata contract for one authored executable state.
pub trait State: Send + Sync + 'static {
    /// Typed input consumed by the state.
    type Input: StructuredValue;
    /// Typed successful output.
    type Output: MfmValue;
    /// Explicit `Never` or inhabited typed failure.
    type Failure: FailureValue;
    /// Typed request authored by a Read/Effect callback; unused by Pure.
    type Request: Serialize + DeserializeOwned + Send + Sync + 'static;
    /// Typed returned observation consumed by Read/Effect settlement; unused by Pure.
    type Returned: Serialize + DeserializeOwned + Send + Sync + 'static;
    /// Typed reviewed safe failure consumed by Read/Effect settlement; unused by Pure.
    type SafeFailure: Serialize + DeserializeOwned + Send + Sync + 'static;
    /// Type-level semantic execution classification.
    type Execution: Execution;
    /// Sealed safe-failure disposition compatible with execution and fallibility.
    type SafeFailureDisposition: SafeFailureDisposition<Self::Execution, Self::Failure>;
    /// Type-level direct or declaratively lowered capability classification.
    type Capability: Capability;

    /// Returns the stable semantic identity of this state behavior.
    fn semantic_state_id() -> Result<StableId>;

    /// Returns the dense bounded durable-fact slots this state may emit.
    fn fact_slots() -> Result<Vec<mfm_spec::CertifiedFactSlot>> {
        Ok(Vec::new())
    }
}

/// Value-only callback frame assembled from one verified current input.
#[derive(Debug, Clone, Copy)]
pub struct StateFrame<'a, Input> {
    input: &'a Input,
}

impl<'a, Input> StateFrame<'a, Input> {
    /// Constructs a frame after the Runtime/store boundary verified its input.
    ///
    /// This value grants neither append nor external-access authority.
    #[doc(hidden)]
    pub const fn from_verified_input(input: &'a Input) -> Self {
        Self { input }
    }

    /// Returns the exact verified state input.
    pub const fn input(self) -> &'a Input {
        self.input
    }
}

/// Exact state-consumable normal observation committed by Runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CommittedObservation<Returned, SafeFailure> {
    /// One schema-valid returned value.
    Returned(Returned),
    /// One reviewed redaction-safe definite failure.
    SafeFailure(SafeFailure),
}

/// Borrowed value-only view over one committed normal observation.
#[derive(Debug, Clone, Copy)]
pub struct CommittedObservationView<'a, Returned, SafeFailure> {
    observation: &'a CommittedObservation<Returned, SafeFailure>,
}

impl<'a, Returned, SafeFailure> CommittedObservationView<'a, Returned, SafeFailure> {
    /// Constructs a view after Runtime has verified the committed observation.
    #[doc(hidden)]
    pub const fn from_committed(
        observation: &'a CommittedObservation<Returned, SafeFailure>,
    ) -> Self {
        Self { observation }
    }

    /// Returns the exact committed normal observation.
    pub const fn observation(self) -> &'a CommittedObservation<Returned, SafeFailure> {
        self.observation
    }
}

/// Closed result of deterministic returned-value settlement.
///
/// Safe-failure settlement cannot produce [`StateSettlement::InvalidEvidence`];
/// disposition-specific proposal types exclude that variant and, under
/// success-only disposition, also exclude typed failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "outcome", rename_all = "snake_case")]
pub enum StateSettlement<Output, Failure> {
    /// One uncommitted typed semantic outcome proposal.
    Proposed(ProposedStateOutcome<Output, Failure>),
    /// The committed returned observation is malformed or inconsistent.
    InvalidEvidence,
}

type PureApply<S> = Arc<
    dyn for<'a> Fn(
            StateFrame<'a, <S as State>::Input>,
        ) -> ProposedStateOutcome<<S as State>::Output, <S as State>::Failure>
        + Send
        + Sync,
>;
type RequestAuthor<S> =
    Arc<dyn for<'a> Fn(StateFrame<'a, <S as State>::Input>) -> <S as State>::Request + Send + Sync>;
type ReturnedObservationSettlement<S> = Arc<
    dyn for<'a> Fn(
            StateFrame<'a, <S as State>::Input>,
            &'a <S as State>::Returned,
        ) -> StateSettlement<<S as State>::Output, <S as State>::Failure>
        + Send
        + Sync,
>;
type SafeFailureObservationSettlement<S> = Arc<
    dyn for<'a> Fn(
            StateFrame<'a, <S as State>::Input>,
            &'a <S as State>::SafeFailure,
        ) -> <<S as State>::SafeFailureDisposition as SafeFailureDisposition<
            <S as State>::Execution,
            <S as State>::Failure,
        >>::SafeFailureProposal<<S as State>::Output>
        + Send
        + Sync,
>;

/// Real typed process callbacks selected for one semantic state implementation.
///
/// Read and Effect bind distinct returned-value and safe-failure settlement
/// callbacks. The safe-failure callback's return type is the disposition's
/// [`SafeFailureDisposition::SafeFailureProposal`], so success-only states cannot
/// construct `Failure` or `InvalidEvidence` for any inhabited safe-failure value.
/// Totality is type-enforced; qualification does not rely on a reviewed sample corpus.
pub enum StructuredStateCallbacks<S: State> {
    /// One deterministic local callback.
    Pure {
        /// Exact typed callback.
        apply: PureApply<S>,
    },
    /// Total request authorship plus deterministic read settlement.
    Read {
        /// Exact typed request callback.
        request: RequestAuthor<S>,
        /// Settlement for one schema-valid returned observation.
        settle_returned: ReturnedObservationSettlement<S>,
        /// Disposition-typed settlement for every inhabited safe-failure value.
        settle_safe_failure: SafeFailureObservationSettlement<S>,
    },
    /// Total request authorship plus deterministic effect settlement.
    Effect {
        /// Exact typed request callback.
        request: RequestAuthor<S>,
        /// Settlement for one schema-valid returned observation.
        settle_returned: ReturnedObservationSettlement<S>,
        /// Disposition-typed settlement for every inhabited safe-failure value.
        settle_safe_failure: SafeFailureObservationSettlement<S>,
    },
}

impl<S: State> std::fmt::Debug for StructuredStateCallbacks<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StructuredStateCallbacks")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}

impl<S: State> StructuredStateCallbacks<S> {
    /// Returns the exact callback execution variant.
    pub const fn kind(&self) -> StructuredExecutionKind {
        match self {
            Self::Pure { .. } => StructuredExecutionKind::Pure,
            Self::Read { .. } => StructuredExecutionKind::Read,
            Self::Effect { .. } => StructuredExecutionKind::Effect,
        }
    }

    /// Invokes the Pure callback, when this is the exact registered variant.
    #[doc(hidden)]
    pub fn invoke_pure(
        &self,
        input: &S::Input,
    ) -> Option<ProposedStateOutcome<S::Output, S::Failure>> {
        match self {
            Self::Pure { apply } => Some(apply(StateFrame::from_verified_input(input))),
            Self::Read { .. } | Self::Effect { .. } => None,
        }
    }

    /// Authors the exact Read/Effect request, when applicable.
    #[doc(hidden)]
    pub fn author_request(&self, input: &S::Input) -> Option<S::Request> {
        match self {
            Self::Read { request, .. } | Self::Effect { request, .. } => {
                Some(request(StateFrame::from_verified_input(input)))
            }
            Self::Pure { .. } => None,
        }
    }

    /// Settles one exact schema-valid returned observation, when applicable.
    #[doc(hidden)]
    pub fn settle_returned(
        &self,
        input: &S::Input,
        returned: &S::Returned,
    ) -> Option<StateSettlement<S::Output, S::Failure>> {
        match self {
            Self::Read {
                settle_returned, ..
            }
            | Self::Effect {
                settle_returned, ..
            } => Some(settle_returned(
                StateFrame::from_verified_input(input),
                returned,
            )),
            Self::Pure { .. } => None,
        }
    }

    /// Settles one exact inhabited safe-failure value, when applicable.
    ///
    /// The callback cannot return `InvalidEvidence`. Under
    /// [`SafeFailureSuccessOnly`], it also cannot return typed failure.
    #[doc(hidden)]
    pub fn settle_safe_failure(
        &self,
        input: &S::Input,
        safe_failure: &S::SafeFailure,
    ) -> Option<StateSettlement<S::Output, S::Failure>> {
        match self {
            Self::Read {
                settle_safe_failure,
                ..
            }
            | Self::Effect {
                settle_safe_failure,
                ..
            } => {
                let proposal = settle_safe_failure(
                    StateFrame::from_verified_input(input),
                    safe_failure,
                );
                Some(S::SafeFailureDisposition::into_settlement(proposal))
            }
            Self::Pure { .. } => None,
        }
    }

    /// Settles one exact committed normal observation, when applicable.
    ///
    /// Returned observations use the full returned settlement. Safe-failure
    /// observations use the disposition-typed success-or-failure proposal path
    /// and never invent an infrastructure semantic failure.
    #[doc(hidden)]
    pub fn settle_observation(
        &self,
        input: &S::Input,
        observation: &CommittedObservation<S::Returned, S::SafeFailure>,
    ) -> Option<StateSettlement<S::Output, S::Failure>> {
        match observation {
            CommittedObservation::Returned(returned) => self.settle_returned(input, returned),
            CommittedObservation::SafeFailure(safe_failure) => {
                self.settle_safe_failure(input, safe_failure)
            }
        }
    }
}

/// Marker for an MFM value whose exact tagged-enum schema is used by `Match`.
pub trait ClosedSum: MfmValue {}

/// Registered pure lexical default mapper from `Source` to `ScopeFailure`.
pub trait DefaultFailureMapper<Source, ScopeFailure>: Send + Sync + 'static
where
    Source: MfmValue,
    ScopeFailure: MfmValue,
{
    /// Exact one-tag mapper output route.
    type Route: ClosedSum;

    /// Exact mapper state selected for this source/scope pair.
    type Mapper: State<Input = Source, Output = Self::Route, Failure = Never>;
}

/// Registered pure custom failure handler for one exact source contract.
pub trait CustomFailureHandler<Source, RecoveredOutput, ScopeFailure>:
    Send + Sync + 'static
where
    Source: MfmValue,
    ScopeFailure: FailureValue,
{
    /// Exact closed handler route type.
    type Route: ClosedSum;

    /// Exact pure infallible handler state.
    type Handler: State<Input = Source, Output = Self::Route, Failure = Never>;

    /// Authors every route through the sealed typed recovery builder.
    fn author_routes<Policy>(
        routes: &mut RecoveryRouteBuilder<RecoveredOutput, ScopeFailure, Policy>,
    ) -> Result<()>
    where
        Policy: AuthoringPolicy;
}

/// Registered child operation definition used by pure substitution.
pub trait ChildOperation: Send + Sync + 'static {
    /// Typed child success value.
    type Output: MfmValue;
    /// Exact child failure value.
    type Failure: FailureValue;

    /// Returns the canonical authored child-program reference.
    fn authored_program_ref() -> Result<ContentRef>;
}

/// Derives the complete semantic contract of a typed state without accepting
/// caller-supplied value or failure contract references.
pub fn state_contract<S: State>() -> Result<StructuredStateContract> {
    StructuredStateContract::new_with_fact_slots(
        S::semantic_state_id()?,
        S::Execution::execution_contract()?,
        S::Input::structured_contract_ref()?,
        structured_value_contract_ref::<S::Output>()?,
        S::Failure::failure_contract()?,
        S::SafeFailureDisposition::contract(),
        S::Capability::requirement_ref()?,
        S::fact_slots()?,
    )
    .map_err(Into::into)
}

/// Derives the exact closed-sum tag and typed payload table from a selector's
/// canonical MFM enum schema.
pub fn closed_sum_contract<T: ClosedSum>() -> Result<ClosedSumContract> {
    let descriptor =
        T::schema_descriptor().map_err(|error| ProgramError::Authoring(error.to_string()))?;
    let SchemaShape::Enum { variants, .. } = &descriptor.identity().shape else {
        return Err(ProgramError::Authoring(
            "Match selector is not a canonical tagged enum".to_owned(),
        ));
    };
    let variants = variants
        .iter()
        .map(|variant| {
            Ok(ClosedSumVariant {
                canonical_tag: variant.name.clone(),
                payloads: closed_sum_payloads(&variant.shape)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    ClosedSumContract::new(structured_value_contract_ref::<T>()?, variants)
        .map_err(|error| ProgramError::Authoring(error.to_string()))
}

fn closed_sum_payloads(shape: &SchemaShape) -> Result<Vec<ClosedSumPayload>> {
    match shape {
        SchemaShape::Unit => Ok(Vec::new()),
        SchemaShape::Struct { fields } => fields
            .iter()
            .map(|field| {
                Ok(ClosedSumPayload {
                    payload_path: vec![derived_stable_id(&field.name)?],
                    contract_ref: inline_value_contract_ref(&field.shape)?,
                })
            })
            .collect(),
        SchemaShape::Tuple(values) => values
            .iter()
            .enumerate()
            .map(|(ordinal, value)| {
                Ok(ClosedSumPayload {
                    payload_path: vec![derived_stable_id(&format!("item-{ordinal}"))?],
                    contract_ref: inline_value_contract_ref(value)?,
                })
            })
            .collect(),
        SchemaShape::InlineValue { .. } => Ok(vec![ClosedSumPayload {
            payload_path: vec![derived_stable_id("value")?],
            contract_ref: inline_value_contract_ref(shape)?,
        }]),
        _ => Err(ProgramError::Authoring(
            "closed-sum payloads must be unit or exact nested MFM values".to_owned(),
        )),
    }
}

fn inline_value_contract_ref(shape: &SchemaShape) -> Result<ContentRef> {
    let SchemaShape::InlineValue {
        schema_id,
        semantic_type_id,
        ..
    } = shape
    else {
        return Err(ProgramError::Authoring(
            "closed-sum payload is not an exact nested MFM value".to_owned(),
        ));
    };
    let contract = RetainedValueContract::new(
        schema_id.clone(),
        semantic_type_id.clone(),
        derived_stable_id("structured-value")?,
        "application/json",
        component_object_evidence_contract_ref()
            .map_err(|error| ProgramError::Authoring(error.to_string()))?,
    )
    .map_err(|error| ProgramError::Authoring(error.to_string()))?;
    retained_value_contract_ref(&contract).map_err(Into::into)
}

fn derived_stable_id(value: &str) -> Result<StableId> {
    StableId::new(value).map_err(|error| ProgramError::Authoring(error.to_string()))
}

/// Typed handle to one already-declared lexical value.
///
/// Producer authority cannot be constructed by a consuming crate:
///
/// ```compile_fail
/// use std::marker::PhantomData;
/// use mfm_program::structured::Value;
/// use mfm_spec::structured::LexicalSlot;
///
/// fn forge<T>(slot: LexicalSlot) -> Value<T> {
///     Value { slot, _value: PhantomData }
/// }
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct Value<T> {
    slot: LexicalSlot,
    _value: PhantomData<fn() -> T>,
}

impl<T> Clone for Value<T> {
    fn clone(&self) -> Self {
        Self {
            slot: self.slot.clone(),
            _value: PhantomData,
        }
    }
}

impl<T> Value<T> {
    /// Returns the exact unresolved lexical slot.
    pub const fn slot(&self) -> &LexicalSlot {
        &self.slot
    }

    /// Binds this exact lexical value to one declared child input root.
    pub fn bind_child(&self, root_id: StableId) -> ChildInputBinding {
        ChildInputBinding {
            root_id,
            slot: self.slot.clone(),
        }
    }

    fn from_slot(slot: LexicalSlot) -> Self {
        Self {
            slot,
            _value: PhantomData,
        }
    }
}

/// One typed caller value selected for a child input root.
///
/// The slot is private so callers cannot manufacture producer authority while
/// assembling a child substitution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildInputBinding {
    root_id: StableId,
    slot: LexicalSlot,
}

/// Nominal value produced by a declaration-ordered collect-all fan-out join.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanOutResults<Output, Failure> {
    declaration_ordered: Vec<LaneOutcome<Output, Failure>>,
}

impl<Output, Failure> FanOutResults<Output, Failure> {
    /// Constructs the nominal join data in certified declaration order.
    ///
    /// This is value data only; certified slot provenance remains the authority
    /// that permits a state input to consume it.
    pub fn from_declaration_ordered(outcomes: Vec<LaneOutcome<Output, Failure>>) -> Self {
        Self {
            declaration_ordered: outcomes,
        }
    }

    /// Returns lane outcomes in certified declaration order.
    pub fn as_slice(&self) -> &[LaneOutcome<Output, Failure>] {
        &self.declaration_ordered
    }

    /// Consumes the nominal join data into its declaration-ordered outcomes.
    pub fn into_inner(self) -> Vec<LaneOutcome<Output, Failure>> {
        self.declaration_ordered
    }
}

impl<Output, Failure> StructuredValue for FanOutResults<Output, Failure>
where
    Output: StructuredValue,
    Failure: FailureValue,
{
    fn structured_contract_ref() -> Result<ContentRef> {
        fan_out_join_contract_ref(
            &Output::structured_contract_ref()?,
            &Failure::failure_contract()?,
        )
        .map_err(Into::into)
    }
}

/// One typed lexical block completion.
pub struct BlockCompletion<Output, Failure> {
    tail: BlockTail,
    _types: PhantomData<fn() -> (Output, Failure)>,
}

impl<Output, Failure> BlockCompletion<Output, Failure> {
    fn normal(value: &Value<Output>) -> Self {
        Self {
            tail: BlockTail::Normal(value.slot.clone()),
            _types: PhantomData,
        }
    }

    fn scope_failure(value: &Value<Failure>) -> Self {
        Self {
            tail: BlockTail::ScopeFailure(value.slot.clone()),
            _types: PhantomData,
        }
    }
}

/// Sealed root authoring policy: all execution kinds and one outer fan-out are available.
pub struct Sequential;

/// Sealed fan-out policy that permits one further nested fan-out.
pub struct FanOutOneRemaining;

/// Sealed fan-out policy at the certified depth-two limit.
pub struct FanOutAtLimit;

/// Sealed type-level policy carried through every nested lexical builder.
pub trait AuthoringPolicy: private::AuthoringPolicySealed + Send + Sync + 'static {}

impl AuthoringPolicy for Sequential {}
impl AuthoringPolicy for FanOutOneRemaining {}
impl AuthoringPolicy for FanOutAtLimit {}

/// Compile-time permission for a state execution class in one lexical policy.
///
/// Every sealed authoring policy admits Reads. Effect permission remains
/// policy-specific so concurrent lexical scopes cannot author direct Effects.
pub trait AllowsExecution<Kind: Execution>: AuthoringPolicy {}

impl AllowsExecution<Pure> for Sequential {}
impl<Capability: RuntimeEffectCapability> AllowsExecution<Effect<Capability>> for Sequential {}
impl AllowsExecution<Pure> for FanOutOneRemaining {}
impl AllowsExecution<Pure> for FanOutAtLimit {}
impl<Policy, Capability> AllowsExecution<Read<Capability>> for Policy
where
    Policy: AuthoringPolicy,
    Capability: RuntimeReadCapability,
{
}

/// Compile-time permission to enter one fan-out and its strictly smaller lane policy.
pub trait AllowsFanOut: AuthoringPolicy {
    /// Policy inherited by every lane of the new fan-out.
    type LanePolicy: AuthoringPolicy;
}

impl AllowsFanOut for Sequential {
    type LanePolicy = FanOutOneRemaining;
}

impl AllowsFanOut for FanOutOneRemaining {
    type LanePolicy = FanOutAtLimit;
}

/// Compile-time permission to consume a policy recipe's affine `proceed` authority.
pub trait AllowsPolicyProceed: AuthoringPolicy {}

impl AllowsPolicyProceed for Sequential {}

/// Declaration-ordered builder for one lexical block.
pub struct BlockBuilder<ScopeFailure: FailureValue, Policy: AuthoringPolicy = Sequential> {
    path: StructuralPath,
    semantic_prefix: Vec<SemanticPathSegment>,
    failure_scope: FailureScopeBinding,
    visible_default_mappers: Vec<FailureMapperRegistration>,
    declarations: Vec<AuthoredDeclaration>,
    labels: BTreeSet<StableId>,
    next_ordinal: u32,
    _types: PhantomData<fn() -> (ScopeFailure, Policy)>,
}

impl<ScopeFailure, Policy> BlockBuilder<ScopeFailure, Policy>
where
    ScopeFailure: FailureValue,
    Policy: AuthoringPolicy,
{
    fn owned(
        path: StructuralPath,
        semantic_prefix: Vec<SemanticPathSegment>,
        scope_id: StableId,
    ) -> Result<Self> {
        Ok(Self {
            path,
            semantic_prefix,
            failure_scope: FailureScopeBinding::Owns {
                scope: FailureScope {
                    scope_id,
                    failure_contract: ScopeFailure::failure_contract()?,
                    default_mappers: Vec::new(),
                },
            },
            visible_default_mappers: Vec::new(),
            declarations: Vec::new(),
            labels: BTreeSet::new(),
            next_ordinal: 0,
            _types: PhantomData,
        })
    }

    fn inherited(
        path: StructuralPath,
        semantic_prefix: Vec<SemanticPathSegment>,
        scope_id: StableId,
        visible_default_mappers: Vec<FailureMapperRegistration>,
    ) -> Result<Self> {
        Ok(Self {
            path,
            semantic_prefix,
            failure_scope: FailureScopeBinding::Inherits {
                scope_id,
                failure_contract: ScopeFailure::failure_contract()?,
            },
            visible_default_mappers,
            declarations: Vec::new(),
            labels: BTreeSet::new(),
            next_ordinal: 0,
            _types: PhantomData,
        })
    }

    /// Registers one exact lexical default mapper in an owned typed scope.
    pub fn failure_map<Source, Mapper>(&mut self) -> Result<()>
    where
        Source: MfmValue,
        ScopeFailure: MfmValue,
        Mapper: DefaultFailureMapper<Source, ScopeFailure>,
    {
        let FailureScopeBinding::Owns { scope } = &mut self.failure_scope else {
            return Err(ProgramError::Authoring(
                "an inherited failure scope cannot install or shadow a mapper".to_owned(),
            ));
        };
        let source = Source::failure_contract()?.contract_ref()?;
        if scope
            .default_mappers
            .iter()
            .any(|registered| registered.source_failure_contract_ref == source)
        {
            return Err(ProgramError::Authoring(
                "duplicate lexical failure mapper".to_owned(),
            ));
        }
        let mapper = state_contract::<Mapper::Mapper>()?;
        validate_pure_never_handler(&mapper, &source)?;
        let route = closed_sum_contract::<Mapper::Route>()?;
        validate_default_route::<ScopeFailure>(&route, &mapper.output_contract_ref)?;
        let registration = FailureMapperRegistration {
            source_failure_contract_ref: source,
            mapper_state_contract_ref: mapper.state_contract_ref,
            route_contract: route,
        };
        scope.default_mappers.push(registration.clone());
        self.visible_default_mappers.push(registration);
        Ok(())
    }

    /// Declares one state call; the returned affine builder must select its exact failure rule.
    pub fn state<'a, S>(
        &'a mut self,
        label: StableId,
        input: &Value<S::Input>,
    ) -> Result<PendingState<'a, S, ScopeFailure, Policy>>
    where
        S: State,
        Policy: AllowsExecution<S::Execution>,
    {
        let contract = state_contract::<S>()?;
        contract.validate()?;
        self.require_visible(&input.slot)?;
        if contract.input_contract_ref != input.slot.contract_ref {
            return Err(ProgramError::Authoring(
                "state input contract does not match the exact lexical slot".to_owned(),
            ));
        }
        let expected_failure = S::Failure::failure_contract()?;
        if expected_failure != contract.failure_contract {
            return Err(ProgramError::Authoring(
                "state failure type and registered failure contract differ".to_owned(),
            ));
        }
        let (path, semantic_path) = self.next_paths(&label, None)?;
        let semantic_call_id = semantic_path.identity()?;
        let output = Value::from_slot(LexicalSlot {
            lexical_path: path,
            contract_ref: contract.output_contract_ref.clone(),
            producer: LexicalProducer::AuthoredCallOutput {
                semantic_call_id: semantic_call_id.clone(),
                role: ResultRole::SuccessOutput,
            },
        });
        Ok(PendingState {
            builder: self,
            label,
            semantic_path,
            semantic_call_id,
            contract,
            inputs: vec![input.slot.clone()],
            output,
            _state: PhantomData,
        })
    }

    /// Declares one child operation with a complete input-root bijection.
    pub fn child<'a, C>(
        &'a mut self,
        label: StableId,
        bindings: Vec<ChildInputBinding>,
    ) -> Result<PendingChild<'a, C, ScopeFailure, Policy>>
    where
        C: ChildOperation,
    {
        let mut root_ids = BTreeSet::new();
        for binding in &bindings {
            self.require_visible(&binding.slot)?;
            if !root_ids.insert(binding.root_id.clone()) {
                return Err(ProgramError::Authoring(
                    "child input root is duplicated".to_owned(),
                ));
            }
        }
        let (path, semantic_path) = self.next_paths(&label, None)?;
        let semantic_call_id = semantic_path.identity()?;
        let output_contract_ref = structured_value_contract_ref::<C::Output>()?;
        let output = Value::from_slot(LexicalSlot {
            lexical_path: path,
            contract_ref: output_contract_ref.clone(),
            producer: LexicalProducer::AuthoredCallOutput {
                semantic_call_id: semantic_call_id.clone(),
                role: ResultRole::SuccessOutput,
            },
        });
        Ok(PendingChild {
            builder: self,
            label,
            semantic_path,
            semantic_call_id,
            child_program_ref: C::authored_program_ref()?,
            input_bindings: bindings
                .into_iter()
                .map(|binding| FragmentInputBinding {
                    child_root_id: binding.root_id,
                    child_contract_ref: binding.slot.contract_ref.clone(),
                    caller_slot: binding.slot,
                })
                .collect(),
            output_contract_ref,
            failure_contract: C::Failure::failure_contract()?,
            output,
            _child: PhantomData,
        })
    }

    fn policy_proceed<Input, Output>(
        &mut self,
        label: StableId,
        input: &Value<Input>,
    ) -> Result<Value<Output>>
    where
        Input: MfmValue,
        Output: MfmValue,
        Policy: AllowsPolicyProceed,
    {
        self.require_visible(&input.slot)?;
        if input.slot.contract_ref != structured_value_contract_ref::<Input>()? {
            return Err(ProgramError::Authoring(
                "policy proceed input contract is not type-derived".to_owned(),
            ));
        }
        let (path, semantic_path) = self.next_paths(&label, None)?;
        let semantic_call_id = semantic_path.identity()?;
        let output_contract_ref = structured_value_contract_ref::<Output>()?;
        let output = Value::from_slot(LexicalSlot {
            lexical_path: path,
            contract_ref: output_contract_ref.clone(),
            producer: LexicalProducer::AuthoredCallOutput {
                semantic_call_id: semantic_call_id.clone(),
                role: ResultRole::SuccessOutput,
            },
        });
        self.declarations
            .push(AuthoredDeclaration::OperationCall(Box::new(
                AuthoredOperationCall {
                    label,
                    semantic_path,
                    semantic_call_id,
                    child_program_ref: policy_proceed_program_ref()?,
                    input_bindings: vec![FragmentInputBinding {
                        child_root_id: derived_stable_id("protected-input")?,
                        child_contract_ref: input.slot.contract_ref.clone(),
                        caller_slot: input.slot.clone(),
                    }],
                    output_contract_ref,
                    failure_contract: ScopeFailure::failure_contract()?,
                    // This directive is deliberately ignored only while the
                    // qualified recipe interpreter substitutes `proceed`.
                    // Ordinary child validation rejects this reserved call.
                    failure_directive: AuthoredFailureDirective::NoFailure,
                    output_slot: output.slot.clone(),
                },
            )));
        Ok(output)
    }

    /// Declares one exhaustive typed `Match` and returns its selected-arm merge value.
    pub fn match_value<Selector, Output>(
        &mut self,
        label: StableId,
        selector: &Value<Selector>,
        author_arms: impl FnOnce(
            &mut MatchBuilder<'_, Selector, Output, ScopeFailure, Policy>,
        ) -> Result<()>,
    ) -> Result<Value<Output>>
    where
        Selector: ClosedSum,
        Output: MfmValue,
    {
        let selector_contract = closed_sum_contract::<Selector>()?;
        let output_contract_ref = structured_value_contract_ref::<Output>()?;
        selector_contract.validate()?;
        self.require_visible(&selector.slot)?;
        if selector.slot.contract_ref != selector_contract.selector_contract_ref {
            return Err(ProgramError::Authoring(
                "Match selector is not the exact registered closed-sum slot".to_owned(),
            ));
        }
        let (path, mut semantic_path) = self.next_paths(&label, None)?;
        let scope_id = self.failure_scope.scope_id().clone();
        let mut match_builder = MatchBuilder {
            path: path.clone(),
            semantic_prefix: semantic_path.segments().to_vec(),
            scope_id,
            visible_default_mappers: self.visible_default_mappers.clone(),
            selector: selector.slot.clone(),
            selector_contract: selector_contract.clone(),
            output_contract_ref: output_contract_ref.clone(),
            arms: Vec::new(),
            _types: PhantomData,
        };
        author_arms(&mut match_builder)?;
        let arms = match_builder.finish()?;
        let normal_slots = arms
            .iter()
            .filter_map(|arm| match &arm.body.tail {
                BlockTail::Normal(slot) => Some(slot.clone()),
                BlockTail::ScopeFailure(_) => None,
            })
            .collect();
        let output = Value::from_slot(LexicalSlot {
            lexical_path: path.clone(),
            contract_ref: output_contract_ref,
            producer: LexicalProducer::MatchMerge {
                match_path: path.clone(),
                declaration_ordered_arm_slots: normal_slots,
            },
        });
        // Prevent an otherwise unused mutable warning while retaining the
        // explicit semantic path in the authored call identity derivation.
        let _ = &mut semantic_path;
        self.declarations
            .push(AuthoredDeclaration::Match(Box::new(AuthoredMatch {
                label,
                selector: selector.slot.clone(),
                selector_contract,
                arms,
                output_slot: output.slot.clone(),
            })));
        Ok(output)
    }

    /// Starts one bounded non-empty collect-all fan-out group.
    pub fn fan_out<'a, Output, LaneFailure>(
        &'a mut self,
        label: StableId,
    ) -> Result<FanOutBuilder<'a, Output, LaneFailure, ScopeFailure, Policy>>
    where
        Output: StructuredValue,
        LaneFailure: FailureValue,
        Policy: AllowsFanOut,
    {
        let (path, semantic_path) = self.next_paths(&label, None)?;
        let lane_output_contract_ref = Output::structured_contract_ref()?;
        let lane_failure_contract = LaneFailure::failure_contract()?;
        let lane_join_contract_ref =
            fan_out_join_contract_ref(&lane_output_contract_ref, &lane_failure_contract)?;
        Ok(FanOutBuilder {
            parent: self,
            label,
            path,
            semantic_prefix: semantic_path.segments().to_vec(),
            lane_output_contract_ref,
            lane_join_contract_ref,
            lane_failure_contract,
            lanes: Vec::new(),
            _types: PhantomData,
        })
    }

    /// Selects a normal lexical block tail without adding a declaration.
    pub fn normal<Output>(
        &self,
        value: &Value<Output>,
    ) -> Result<BlockCompletion<Output, ScopeFailure>> {
        self.require_visible(&value.slot)?;
        Ok(BlockCompletion::normal(value))
    }

    /// Selects the current typed lexical failure tail without adding a declaration.
    pub fn scope_failure<Output>(
        &self,
        value: &Value<ScopeFailure>,
    ) -> Result<BlockCompletion<Output, ScopeFailure>>
    where
        ScopeFailure: MfmValue,
    {
        self.require_visible(&value.slot)?;
        let expected = ScopeFailure::failure_contract()?.contract_ref()?;
        if value.slot.contract_ref != expected {
            return Err(ProgramError::Authoring(
                "scope failure tail contract is not exact".to_owned(),
            ));
        }
        Ok(BlockCompletion::scope_failure(value))
    }

    fn next_paths(
        &mut self,
        label: &StableId,
        discriminator: Option<StableId>,
    ) -> Result<(StructuralPath, SemanticCallPath)> {
        if !self.labels.insert(label.clone()) {
            return Err(ProgramError::Authoring(
                "duplicate stable declaration label in one lexical block".to_owned(),
            ));
        }
        let ordinal = self.next_ordinal;
        self.next_ordinal = self
            .next_ordinal
            .checked_add(1)
            .ok_or_else(|| ProgramError::Authoring("declaration ordinal exceeds u32".to_owned()))?;
        let path = self.path.child(StructuralPathSegment::Declaration {
            label: label.clone(),
            ordinal,
        })?;
        let mut semantic = self.semantic_prefix.clone();
        semantic.push(SemanticPathSegment {
            label: label.clone(),
            discriminator,
        });
        Ok((path, SemanticCallPath::new(semantic)?))
    }

    fn require_visible(&self, slot: &LexicalSlot) -> Result<()> {
        if !lexically_visible(slot, &self.path, self.next_ordinal) {
            return Err(ProgramError::Authoring(
                "lexical value does not dominate this block tail".to_owned(),
            ));
        }
        Ok(())
    }

    fn into_block<Output>(
        self,
        completion: BlockCompletion<Output, ScopeFailure>,
    ) -> Result<AuthoredBlock> {
        let selected = match &completion.tail {
            BlockTail::Normal(slot) | BlockTail::ScopeFailure(slot) => slot,
        };
        self.require_visible(selected)?;
        Ok(AuthoredBlock {
            path: self.path,
            failure_scope: self.failure_scope,
            declarations: self.declarations,
            tail: completion.tail,
        })
    }
}

/// Affine unresolved state call that must select exactly one failure directive.
#[must_use = "a state is authored only after selecting its exact failure directive"]
pub struct PendingState<'a, S, ScopeFailure, Policy = Sequential>
where
    S: State,
    ScopeFailure: FailureValue,
    Policy: AuthoringPolicy,
{
    builder: &'a mut BlockBuilder<ScopeFailure, Policy>,
    label: StableId,
    semantic_path: SemanticCallPath,
    semantic_call_id: mfm_ids::SemanticCallId,
    contract: StructuredStateContract,
    inputs: Vec<LexicalSlot>,
    output: Value<S::Output>,
    _state: PhantomData<fn() -> S>,
}

impl<S, ScopeFailure, Policy> PendingState<'_, S, ScopeFailure, Policy>
where
    S: State<Failure = Never>,
    ScopeFailure: FailureValue,
    Policy: AuthoringPolicy,
{
    /// Completes an exact `Never` state call with no failure plan.
    pub fn infallible(self) -> Result<Value<S::Output>> {
        self.finish(AuthoredFailureDirective::NoFailure)
    }
}

impl<S, ScopeFailure, Policy> PendingState<'_, S, ScopeFailure, Policy>
where
    S: State,
    S::Failure: MfmValue,
    ScopeFailure: MfmValue,
    Policy: AuthoringPolicy,
{
    /// Selects exactly one matching lexical default mapper.
    pub fn or_default(self) -> Result<Value<S::Output>> {
        let source = S::Failure::failure_contract()?.contract_ref()?;
        let matching = self
            .builder
            .visible_default_mappers
            .iter()
            .filter(|entry| entry.source_failure_contract_ref == source)
            .count();
        if matching != 1 {
            return Err(ProgramError::Authoring(
                ".or_default() requires exactly one matching lexical mapper".to_owned(),
            ));
        }
        self.finish(AuthoredFailureDirective::Default)
    }
}

impl<S, ScopeFailure, Policy> PendingState<'_, S, ScopeFailure, Policy>
where
    S: State,
    S::Failure: MfmValue,
    ScopeFailure: FailureValue,
    Policy: AuthoringPolicy,
{
    /// Selects one registered custom pure handler and exact closed route table.
    pub fn on_failure<Handler>(self) -> Result<Value<S::Output>>
    where
        Handler: CustomFailureHandler<S::Failure, S::Output, ScopeFailure>,
    {
        let handler = state_contract::<Handler::Handler>()?;
        let source = S::Failure::failure_contract()?.contract_ref()?;
        validate_pure_never_handler(&handler, &source)?;
        let route = closed_sum_contract::<Handler::Route>()?;
        route.validate()?;
        if route.selector_contract_ref != handler.output_contract_ref {
            return Err(ProgramError::Authoring(
                "custom handler route and output contracts differ".to_owned(),
            ));
        }
        let arms = author_recovery_routes::<Handler, S::Failure, S::Output, ScopeFailure, Policy>(
            &self.semantic_path,
            &self.semantic_call_id,
            &self.output.slot.lexical_path,
            &self.builder.failure_scope,
            &self.builder.visible_default_mappers,
            &handler.output_contract_ref,
            &route,
        )?;
        self.finish(AuthoredFailureDirective::Custom {
            handler_state_contract_ref: Box::new(handler.state_contract_ref),
            route_contract: Box::new(route),
            arms,
        })
    }
}

impl<S, ScopeFailure, Policy> PendingState<'_, S, ScopeFailure, Policy>
where
    S: State,
    ScopeFailure: FailureValue,
    Policy: AuthoringPolicy,
{
    fn finish(self, failure_directive: AuthoredFailureDirective) -> Result<Value<S::Output>> {
        self.builder
            .declarations
            .push(AuthoredDeclaration::State(Box::new(AuthoredStateCall {
                label: self.label,
                semantic_path: self.semantic_path,
                semantic_call_id: self.semantic_call_id,
                contract: self.contract,
                inputs: self.inputs,
                failure_directive,
                output_slot: self.output.slot.clone(),
            })));
        Ok(self.output)
    }
}

/// Affine unresolved child call that must select exactly one failure directive.
#[must_use = "a child call is authored only after selecting its exact failure directive"]
pub struct PendingChild<'a, C, ScopeFailure, Policy = Sequential>
where
    C: ChildOperation,
    ScopeFailure: FailureValue,
    Policy: AuthoringPolicy,
{
    builder: &'a mut BlockBuilder<ScopeFailure, Policy>,
    label: StableId,
    semantic_path: SemanticCallPath,
    semantic_call_id: mfm_ids::SemanticCallId,
    child_program_ref: ContentRef,
    input_bindings: Vec<FragmentInputBinding>,
    output_contract_ref: ContentRef,
    failure_contract: StructuredFailureContract,
    output: Value<C::Output>,
    _child: PhantomData<fn() -> C>,
}

impl<C, ScopeFailure, Policy> PendingChild<'_, C, ScopeFailure, Policy>
where
    C: ChildOperation<Failure = Never>,
    ScopeFailure: FailureValue,
    Policy: AuthoringPolicy,
{
    /// Completes an exact `Never` child boundary.
    pub fn infallible(self) -> Result<Value<C::Output>> {
        self.finish(AuthoredFailureDirective::NoFailure)
    }
}

impl<C, ScopeFailure, Policy> PendingChild<'_, C, ScopeFailure, Policy>
where
    C: ChildOperation,
    C::Failure: MfmValue,
    ScopeFailure: MfmValue,
    Policy: AuthoringPolicy,
{
    /// Selects the one exact lexical default mapper for the child boundary.
    pub fn or_default(self) -> Result<Value<C::Output>> {
        let source = C::Failure::failure_contract()?.contract_ref()?;
        let matching = self
            .builder
            .visible_default_mappers
            .iter()
            .filter(|entry| entry.source_failure_contract_ref == source)
            .count();
        if matching != 1 {
            return Err(ProgramError::Authoring(
                ".or_default() requires exactly one matching lexical mapper".to_owned(),
            ));
        }
        self.finish(AuthoredFailureDirective::Default)
    }
}

impl<C, ScopeFailure, Policy> PendingChild<'_, C, ScopeFailure, Policy>
where
    C: ChildOperation,
    C::Failure: MfmValue,
    ScopeFailure: FailureValue,
    Policy: AuthoringPolicy,
{
    /// Selects one registered custom pure child-boundary handler.
    pub fn on_failure<Handler>(self) -> Result<Value<C::Output>>
    where
        Handler: CustomFailureHandler<C::Failure, C::Output, ScopeFailure>,
    {
        let handler = state_contract::<Handler::Handler>()?;
        let source = C::Failure::failure_contract()?.contract_ref()?;
        validate_pure_never_handler(&handler, &source)?;
        let route = closed_sum_contract::<Handler::Route>()?;
        route.validate()?;
        if route.selector_contract_ref != handler.output_contract_ref {
            return Err(ProgramError::Authoring(
                "custom handler route and output contracts differ".to_owned(),
            ));
        }
        let arms = author_recovery_routes::<Handler, C::Failure, C::Output, ScopeFailure, Policy>(
            &self.semantic_path,
            &self.semantic_call_id,
            &self.output.slot.lexical_path,
            &self.builder.failure_scope,
            &self.builder.visible_default_mappers,
            &handler.output_contract_ref,
            &route,
        )?;
        self.finish(AuthoredFailureDirective::Custom {
            handler_state_contract_ref: Box::new(handler.state_contract_ref),
            route_contract: Box::new(route),
            arms,
        })
    }
}

impl<C, ScopeFailure, Policy> PendingChild<'_, C, ScopeFailure, Policy>
where
    C: ChildOperation,
    ScopeFailure: FailureValue,
    Policy: AuthoringPolicy,
{
    fn finish(self, failure_directive: AuthoredFailureDirective) -> Result<Value<C::Output>> {
        self.builder
            .declarations
            .push(AuthoredDeclaration::OperationCall(Box::new(
                AuthoredOperationCall {
                    label: self.label,
                    semantic_path: self.semantic_path,
                    semantic_call_id: self.semantic_call_id,
                    child_program_ref: self.child_program_ref,
                    input_bindings: self.input_bindings,
                    output_contract_ref: self.output_contract_ref,
                    failure_contract: self.failure_contract,
                    failure_directive,
                    output_slot: self.output.slot.clone(),
                },
            )));
        Ok(self.output)
    }
}

/// Exact payload table visible while authoring one selected custom recovery arm.
pub struct VariantPayloads {
    selector: LexicalSlot,
    canonical_tag: String,
    payloads: Vec<mfm_spec::structured::ClosedSumPayload>,
    arm_path: StructuralPath,
}

impl VariantPayloads {
    /// Selects one exact registered payload as a typed lexical value.
    pub fn value<T: MfmValue>(&self, payload_path: &[StableId]) -> Result<Value<T>> {
        let contract_ref = structured_value_contract_ref::<T>()?;
        let payload = self
            .payloads
            .iter()
            .find(|payload| payload.payload_path == payload_path)
            .ok_or_else(|| {
                ProgramError::Authoring(
                    "custom recovery payload path is not registered for this tag".to_owned(),
                )
            })?;
        if payload.contract_ref != contract_ref {
            return Err(ProgramError::Authoring(
                "custom recovery payload contract is not exact".to_owned(),
            ));
        }
        Ok(Value::from_slot(LexicalSlot {
            lexical_path: self.arm_path.clone(),
            contract_ref,
            producer: LexicalProducer::VariantPayload {
                selector: Box::new(self.selector.clone()),
                canonical_tag: self.canonical_tag.clone(),
                payload_path: payload_path.to_vec(),
            },
        }))
    }
}

/// Sealed authoring surface for an exhaustive custom failure-handler route.
pub struct RecoveryRouteBuilder<Output, ScopeFailure, Policy = Sequential>
where
    ScopeFailure: FailureValue,
    Policy: AuthoringPolicy,
{
    plan_path: StructuralPath,
    semantic_prefix: Vec<SemanticPathSegment>,
    scope_id: StableId,
    visible_default_mappers: Vec<FailureMapperRegistration>,
    route_contract: ClosedSumContract,
    handler_selector: LexicalSlot,
    arms: Vec<AuthoredMatchArm>,
    _output: PhantomData<fn() -> Output>,
    _scope_failure: PhantomData<fn() -> ScopeFailure>,
    _policy: PhantomData<fn() -> Policy>,
}

impl<Output, ScopeFailure, Policy> RecoveryRouteBuilder<Output, ScopeFailure, Policy>
where
    ScopeFailure: FailureValue,
    Policy: AuthoringPolicy,
{
    /// Adds one route arm in the route contract's canonical tag order.
    pub fn arm(
        &mut self,
        canonical_tag: impl Into<String>,
        label: StableId,
        author: impl FnOnce(
            &mut BlockBuilder<ScopeFailure, Policy>,
            &VariantPayloads,
        ) -> Result<BlockCompletion<Output, ScopeFailure>>,
    ) -> Result<()> {
        let canonical_tag = canonical_tag.into();
        if self
            .arms
            .iter()
            .any(|arm| arm.canonical_tag == canonical_tag || arm.label == label)
        {
            return Err(ProgramError::Authoring(
                "duplicate custom recovery tag or stable arm label".to_owned(),
            ));
        }
        let variant = self
            .route_contract
            .variants
            .iter()
            .find(|variant| variant.canonical_tag == canonical_tag)
            .cloned()
            .ok_or_else(|| {
                ProgramError::Authoring(
                    "custom recovery tag is not present in the registered route".to_owned(),
                )
            })?;
        let path = self.plan_path.child(StructuralPathSegment::MatchArm {
            label: label.clone(),
            tag: canonical_tag.clone(),
        })?;
        let mut semantic = self.semantic_prefix.clone();
        semantic.push(SemanticPathSegment {
            label: label.clone(),
            discriminator: Some(label.clone()),
        });
        let mut block = BlockBuilder::inherited(
            path.clone(),
            semantic,
            self.scope_id.clone(),
            self.visible_default_mappers.clone(),
        )?;
        let payloads = VariantPayloads {
            selector: self.handler_selector.clone(),
            canonical_tag: canonical_tag.clone(),
            payloads: variant.payloads,
            arm_path: path.clone(),
        };
        let completion = author(&mut block, &payloads)?;
        let body = block.into_block(completion)?;
        self.arms.push(AuthoredMatchArm {
            canonical_tag,
            label,
            body,
        });
        Ok(())
    }

    fn finish(self) -> Result<Vec<AuthoredMatchArm>> {
        validate_route_arms(&self.route_contract, &self.arms)?;
        Ok(self.arms)
    }
}

#[allow(clippy::too_many_arguments)]
fn author_recovery_routes<Handler, Source, Output, ScopeFailure, Policy>(
    semantic_path: &SemanticCallPath,
    semantic_call_id: &mfm_ids::SemanticCallId,
    occurrence_path: &StructuralPath,
    failure_scope: &FailureScopeBinding,
    visible_default_mappers: &[FailureMapperRegistration],
    handler_output_contract_ref: &ContentRef,
    route_contract: &ClosedSumContract,
) -> Result<Vec<AuthoredMatchArm>>
where
    Source: MfmValue,
    ScopeFailure: FailureValue,
    Handler: CustomFailureHandler<Source, Output, ScopeFailure>,
    Policy: AuthoringPolicy,
{
    let handler_label =
        StableId::new("handler").map_err(|error| ProgramError::Authoring(error.to_string()))?;
    let plan_path = occurrence_path.child(StructuralPathSegment::FailurePlan {
        label: handler_label.clone(),
    })?;
    let handler_path = plan_path.child(StructuralPathSegment::Declaration {
        label: handler_label,
        ordinal: 0,
    })?;
    let handler_selector = LexicalSlot {
        lexical_path: handler_path,
        contract_ref: handler_output_contract_ref.clone(),
        producer: LexicalProducer::AuthoredCallOutput {
            semantic_call_id: failure_handler_semantic_call_id(semantic_call_id)?,
            role: ResultRole::SuccessOutput,
        },
    };
    let mut routes: RecoveryRouteBuilder<Output, ScopeFailure, Policy> = RecoveryRouteBuilder {
        plan_path,
        semantic_prefix: semantic_path.segments().to_vec(),
        scope_id: failure_scope.scope_id().clone(),
        visible_default_mappers: visible_default_mappers.to_vec(),
        route_contract: route_contract.clone(),
        handler_selector,
        arms: Vec::new(),
        _output: PhantomData,
        _scope_failure: PhantomData,
        _policy: PhantomData,
    };
    Handler::author_routes(&mut routes)?;
    routes.finish()
}

/// Builder for one exact exhaustive `Match` arm table.
pub struct MatchBuilder<'a, Selector, Output, ScopeFailure, Policy = Sequential>
where
    Selector: ClosedSum,
    ScopeFailure: FailureValue,
    Policy: AuthoringPolicy,
{
    path: StructuralPath,
    semantic_prefix: Vec<SemanticPathSegment>,
    scope_id: StableId,
    visible_default_mappers: Vec<FailureMapperRegistration>,
    selector: LexicalSlot,
    selector_contract: ClosedSumContract,
    output_contract_ref: ContentRef,
    arms: Vec<AuthoredMatchArm>,
    _types: PhantomData<&'a (Selector, Output, ScopeFailure, Policy)>,
}

impl<Selector, Output, ScopeFailure, Policy>
    MatchBuilder<'_, Selector, Output, ScopeFailure, Policy>
where
    Selector: ClosedSum,
    ScopeFailure: FailureValue,
    Policy: AuthoringPolicy,
{
    /// Adds one exact canonical tag arm.
    pub fn arm(
        &mut self,
        canonical_tag: impl Into<String>,
        label: StableId,
        author: impl FnOnce(
            &mut BlockBuilder<ScopeFailure, Policy>,
            &VariantPayloads,
        ) -> Result<BlockCompletion<Output, ScopeFailure>>,
    ) -> Result<()> {
        let canonical_tag = canonical_tag.into();
        if self
            .arms
            .iter()
            .any(|arm| arm.canonical_tag == canonical_tag || arm.label == label)
        {
            return Err(ProgramError::Authoring(
                "duplicate Match tag or stable arm label".to_owned(),
            ));
        }
        let path = self.path.child(StructuralPathSegment::MatchArm {
            label: label.clone(),
            tag: canonical_tag.clone(),
        })?;
        let mut semantic = self.semantic_prefix.clone();
        semantic.push(SemanticPathSegment {
            label: label.clone(),
            discriminator: Some(label.clone()),
        });
        let mut block = BlockBuilder::inherited(
            path.clone(),
            semantic,
            self.scope_id.clone(),
            self.visible_default_mappers.clone(),
        )?;
        let variant = self
            .selector_contract
            .variants
            .iter()
            .find(|variant| variant.canonical_tag == canonical_tag)
            .cloned()
            .ok_or_else(|| {
                ProgramError::Authoring(
                    "Match arm tag is not present in the registered selector".to_owned(),
                )
            })?;
        let payloads = VariantPayloads {
            selector: self.selector.clone(),
            canonical_tag: canonical_tag.clone(),
            payloads: variant.payloads,
            arm_path: path,
        };
        let completion = author(&mut block, &payloads)?;
        if let BlockTail::Normal(slot) = &completion.tail {
            if slot.contract_ref != self.output_contract_ref {
                return Err(ProgramError::Authoring(
                    "continuing Match arm result contract differs".to_owned(),
                ));
            }
        }
        let body = block.into_block(completion)?;
        self.arms.push(AuthoredMatchArm {
            canonical_tag,
            label,
            body,
        });
        Ok(())
    }

    fn finish(self) -> Result<Vec<AuthoredMatchArm>> {
        let expected: Vec<&str> = self
            .selector_contract
            .variants
            .iter()
            .map(|variant| variant.canonical_tag.as_str())
            .collect();
        let actual: Vec<&str> = self
            .arms
            .iter()
            .map(|arm| arm.canonical_tag.as_str())
            .collect();
        if actual != expected {
            return Err(ProgramError::Authoring(format!(
                "Match arms are not exhaustive in the registered canonical tag order: expected {expected:?}, got {actual:?}"
            )));
        }
        Ok(self.arms)
    }
}

/// Builder for one non-empty declaration-ordered fan-out group.
#[must_use = "a fan-out is authored only after adding lanes and finishing its join"]
pub struct FanOutBuilder<'a, Output, LaneFailure, ScopeFailure, Policy = Sequential>
where
    LaneFailure: FailureValue,
    ScopeFailure: FailureValue,
    Policy: AllowsFanOut,
{
    parent: &'a mut BlockBuilder<ScopeFailure, Policy>,
    label: StableId,
    path: StructuralPath,
    semantic_prefix: Vec<SemanticPathSegment>,
    lane_output_contract_ref: ContentRef,
    lane_join_contract_ref: ContentRef,
    lane_failure_contract: StructuredFailureContract,
    lanes: Vec<AuthoredFanOutLane>,
    _types: PhantomData<fn() -> (Output, LaneFailure)>,
}

impl<Output, LaneFailure, ScopeFailure, Policy>
    FanOutBuilder<'_, Output, LaneFailure, ScopeFailure, Policy>
where
    LaneFailure: FailureValue,
    ScopeFailure: FailureValue,
    Policy: AllowsFanOut,
{
    /// Adds one lane in semantic declaration order.
    pub fn lane(
        &mut self,
        key: StableId,
        author: impl FnOnce(
            &mut BlockBuilder<LaneFailure, Policy::LanePolicy>,
        ) -> Result<BlockCompletion<Output, LaneFailure>>,
    ) -> Result<()> {
        if self.lanes.iter().any(|lane| lane.key == key) {
            return Err(ProgramError::Authoring(
                "duplicate fan-out lane key".to_owned(),
            ));
        }
        let declaration_ordinal = u32::try_from(self.lanes.len())
            .map_err(|_| ProgramError::Authoring("fan-out lane count exceeds u32".to_owned()))?;
        let path = self.path.child(StructuralPathSegment::FanOutLane {
            key: key.clone(),
            ordinal: declaration_ordinal,
        })?;
        let mut semantic = self.semantic_prefix.clone();
        semantic.push(SemanticPathSegment {
            label: key.clone(),
            discriminator: Some(key.clone()),
        });
        let scope_id = StableId::new(format!("lane/{}/{}", self.label.as_str(), key.as_str()))
            .map_err(|error| ProgramError::Authoring(error.to_string()))?;
        let mut block =
            BlockBuilder::<LaneFailure, Policy::LanePolicy>::owned(path, semantic, scope_id)?;
        let completion = author(&mut block)?;
        if let BlockTail::Normal(slot) = &completion.tail {
            if slot.contract_ref != self.lane_output_contract_ref {
                return Err(ProgramError::Authoring(
                    "fan-out lane output contract differs".to_owned(),
                ));
            }
        }
        let body = block.into_block(completion)?;
        self.lanes.push(AuthoredFanOutLane {
            key,
            declaration_ordinal,
            body,
        });
        Ok(())
    }

    /// Completes the group and returns its nominal declaration-ordered join value.
    pub fn finish(self) -> Result<Value<FanOutResults<Output, LaneFailure>>> {
        if self.lanes.is_empty() {
            return Err(ProgramError::Authoring(
                "FanOut must contain at least one lane".to_owned(),
            ));
        }
        let lane_slots = self
            .lanes
            .iter()
            .map(|lane| match &lane.body.tail {
                BlockTail::Normal(slot) | BlockTail::ScopeFailure(slot) => slot.clone(),
            })
            .collect();
        let output = Value::from_slot(LexicalSlot {
            lexical_path: self.path.clone(),
            contract_ref: self.lane_join_contract_ref,
            producer: LexicalProducer::FanOutJoin {
                group_path: self.path.clone(),
                declaration_ordered_lane_slots: lane_slots,
            },
        });
        self.parent
            .declarations
            .push(AuthoredDeclaration::FanOut(Box::new(AuthoredFanOut {
                label: self.label,
                lane_output_contract_ref: self.lane_output_contract_ref,
                lane_failure_contract: self.lane_failure_contract,
                lanes: self.lanes,
                output_slot: output.slot.clone(),
            })));
        Ok(output)
    }
}

/// Non-cloneable structural authority for the one affine protected boundary
/// inside a policy expansion recipe.
///
/// ```compile_fail
/// use mfm_program::structured::PolicyProceed;
///
/// fn duplicate<I, O, F>(proceed: PolicyProceed<I, O, F>) {
///     let _second = proceed.clone();
/// }
/// ```
#[must_use = "the policy recipe must structurally consume its proceed authority once"]
pub struct PolicyProceed<Input, Output, Failure> {
    _input: PhantomData<fn() -> Input>,
    _output: PhantomData<fn() -> Output>,
    _failure: PhantomData<fn() -> Failure>,
}

impl<Input, Output, Failure> PolicyProceed<Input, Output, Failure>
where
    Input: MfmValue,
    Output: MfmValue,
    Failure: FailureValue,
{
    /// Inserts the protected boundary at this exact declaration position and
    /// consumes the sole authoring authority.
    pub fn call(
        self,
        block: &mut BlockBuilder<Failure>,
        label: StableId,
        input: &Value<Input>,
    ) -> Result<Value<Output>> {
        block.policy_proceed(label, input)
    }
}

/// Canonical callback-free policy expansion recipe with one sealed `proceed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyExpansionRecipe {
    program: AuthoredStructuredProgram,
    failure_post: Option<AuthoredStructuredProgram>,
}

impl PolicyExpansionRecipe {
    /// Returns the exact canonical authored recipe program.
    pub const fn program(&self) -> &AuthoredStructuredProgram {
        &self.program
    }

    /// Returns the optional callback-free failure-post program attached to the
    /// exact protected failure continuation.
    pub const fn failure_post(&self) -> Option<&AuthoredStructuredProgram> {
        self.failure_post.as_ref()
    }

    /// Returns the canonical recipe identity used by a policy contract.
    pub fn content_ref(&self) -> Result<ContentRef> {
        policy_expansion_recipe_ref(self).map_err(Into::into)
    }
}

/// Opaque typed failure-post recipe that preserves the protected failure on
/// normal completion. Any newly committed post-state failure follows the
/// post program's own exact failure plans.
pub struct PolicyFailurePostRecipe<ProtectedFailure: MfmValue> {
    program: AuthoredStructuredProgram,
    _failure: PhantomData<fn() -> ProtectedFailure>,
}

/// Typed authoring builder for ordinary states that run only after the exact
/// protected typed failure and before its enclosing fragment propagation.
pub struct PolicyFailurePostBuilder<ProtectedFailure, PostFailure>
where
    ProtectedFailure: MfmValue,
    PostFailure: FailureValue,
{
    operation: OperationBuilder<ProtectedFailure, PostFailure>,
    protected_failure: Value<ProtectedFailure>,
}

impl<ProtectedFailure, PostFailure> PolicyFailurePostBuilder<ProtectedFailure, PostFailure>
where
    ProtectedFailure: MfmValue,
    PostFailure: FailureValue,
{
    /// Starts a failure-post program and returns the exact protected-failure
    /// input available to its ordinary support states.
    pub fn new(recipe_id: StableId, scope_id: StableId) -> Result<(Self, Value<ProtectedFailure>)> {
        let mut operation = OperationBuilder::new(recipe_id, scope_id)?;
        let protected_failure = operation.input(derived_stable_id("protected-failure")?)?;
        Ok((
            Self {
                operation,
                protected_failure: protected_failure.clone(),
            },
            protected_failure,
        ))
    }

    /// Returns the failure-post program's declaration-ordered root builder.
    pub const fn root(&mut self) -> &mut BlockBuilder<PostFailure> {
        self.operation.root()
    }

    /// Finishes the recipe with the original protected failure as its exact
    /// normal tail. Support-state failures retain their own authored plans.
    pub fn finish(mut self) -> Result<PolicyFailurePostRecipe<ProtectedFailure>> {
        let completion = self.operation.succeed(&self.protected_failure)?;
        let program = self.operation.finish(completion)?;
        Ok(PolicyFailurePostRecipe {
            program,
            _failure: PhantomData,
        })
    }

    /// Finishes the recipe with an explicitly mapped protected failure.
    ///
    /// Certification accepts only a contiguous terminal chain of ordinary
    /// `Pure + Never` states from the original protected input to this exact
    /// mapped tail. Those occurrences become the affine propagation mapping
    /// chain rather than pre-boundary support declarations.
    pub fn finish_with_mapped_failure(
        mut self,
        mapped_failure: &Value<ProtectedFailure>,
    ) -> Result<PolicyFailurePostRecipe<ProtectedFailure>> {
        if mapped_failure.slot == self.protected_failure.slot {
            return Err(ProgramError::Authoring(
                "an explicit failure mapping must select a distinct state output".to_owned(),
            ));
        }
        let completion = self.operation.succeed(mapped_failure)?;
        let program = self.operation.finish(completion)?;
        Ok(PolicyFailurePostRecipe {
            program,
            _failure: PhantomData,
        })
    }
}

/// Typed authoring builder for one boundary-specific policy recipe.
pub struct PolicyRecipeBuilder<Input, Output, Failure>
where
    Input: MfmValue,
    Output: MfmValue,
    Failure: FailureValue,
{
    operation: OperationBuilder<Output, Failure>,
    _input: PhantomData<fn() -> Input>,
}

impl<Input, Output, Failure> PolicyRecipeBuilder<Input, Output, Failure>
where
    Input: MfmValue,
    Output: MfmValue,
    Failure: FailureValue,
{
    /// Starts one recipe and mints its sole non-cloneable `proceed` authority.
    #[allow(clippy::type_complexity)]
    pub fn new(
        recipe_id: StableId,
        scope_id: StableId,
    ) -> Result<(Self, Value<Input>, PolicyProceed<Input, Output, Failure>)> {
        let mut operation = OperationBuilder::new(recipe_id, scope_id)?;
        let input = operation.input(derived_stable_id("protected-input")?)?;
        Ok((
            Self {
                operation,
                _input: PhantomData,
            },
            input,
            PolicyProceed {
                _input: PhantomData,
                _output: PhantomData,
                _failure: PhantomData,
            },
        ))
    }

    /// Returns the recipe's declaration-ordered root builder.
    pub const fn root(&mut self) -> &mut BlockBuilder<Failure> {
        self.operation.root()
    }

    /// Selects the recipe's exact successful boundary value.
    pub fn succeed(&mut self, value: &Value<Output>) -> Result<BlockCompletion<Output, Failure>> {
        self.operation.succeed(value)
    }

    /// Selects the recipe's exact typed failure boundary value.
    pub fn fail(&mut self, value: &Value<Failure>) -> Result<BlockCompletion<Output, Failure>>
    where
        Failure: MfmValue,
    {
        self.operation.fail(value)
    }

    /// Finishes the callback-free recipe and proves exactly one structural
    /// `proceed` placeholder was authored.
    pub fn finish(
        self,
        completion: BlockCompletion<Output, Failure>,
    ) -> Result<PolicyExpansionRecipe> {
        self.finish_recipe(completion, None)
    }

    /// Finishes a typed wrapper and attaches one callback-free failure-post
    /// program to the exact affine protected-failure path.
    pub fn finish_with_failure_post(
        self,
        completion: BlockCompletion<Output, Failure>,
        failure_post: PolicyFailurePostRecipe<Failure>,
    ) -> Result<PolicyExpansionRecipe>
    where
        Failure: MfmValue,
    {
        self.finish_recipe(completion, Some(failure_post.program))
    }

    fn finish_recipe(
        self,
        completion: BlockCompletion<Output, Failure>,
        failure_post: Option<AuthoredStructuredProgram>,
    ) -> Result<PolicyExpansionRecipe> {
        let program = self.operation.finish(completion)?;
        if count_policy_proceeds(&program.root)? != 1 {
            return Err(ProgramError::Authoring(
                "policy recipe must contain exactly one affine proceed declaration".to_owned(),
            ));
        }
        Ok(PolicyExpansionRecipe {
            program,
            failure_post,
        })
    }
}

/// Root operation authoring builder.
pub struct OperationBuilder<Output, Failure>
where
    Output: StructuredValue,
    Failure: FailureValue,
{
    operation_id: StableId,
    output_contract_ref: ContentRef,
    input_roots: Vec<LexicalSlot>,
    root: BlockBuilder<Failure>,
    root_completion: Option<BlockTail>,
    _output: PhantomData<fn() -> Output>,
}

impl<Output, Failure> OperationBuilder<Output, Failure>
where
    Output: StructuredValue,
    Failure: FailureValue,
{
    /// Starts one structured operation root.
    pub fn new(operation_id: StableId, scope_id: StableId) -> Result<Self> {
        let root_path = StructuralPath::new(vec![StructuralPathSegment::Root {
            operation_id: operation_id.clone(),
        }])?;
        Ok(Self {
            operation_id,
            output_contract_ref: Output::structured_contract_ref()?,
            input_roots: Vec::new(),
            root: BlockBuilder::owned(root_path, Vec::new(), scope_id)?,
            root_completion: None,
            _output: PhantomData,
        })
    }

    /// Declares one immutable admission input root.
    pub fn input<T: MfmValue>(&mut self, root_id: StableId) -> Result<Value<T>> {
        if self.input_roots.iter().any(
            |slot| matches!(&slot.producer, LexicalProducer::AdmissionRoot { root_id: existing } if existing == &root_id),
        ) {
            return Err(ProgramError::Authoring(
                "duplicate admission input root".to_owned(),
            ));
        }
        let slot = LexicalSlot {
            lexical_path: self.root.path.clone(),
            contract_ref: structured_value_contract_ref::<T>()?,
            producer: LexicalProducer::AdmissionRoot { root_id },
        };
        self.input_roots.push(slot.clone());
        Ok(Value::from_slot(slot))
    }

    /// Returns the declaration-ordered root block builder.
    pub const fn root(&mut self) -> &mut BlockBuilder<Failure> {
        &mut self.root
    }

    /// Selects root success without appending an instruction or occurrence.
    pub fn succeed(&mut self, value: &Value<Output>) -> Result<BlockCompletion<Output, Failure>> {
        if value.slot.contract_ref != self.output_contract_ref {
            return Err(ProgramError::Authoring(
                "root success contract differs from the operation contract".to_owned(),
            ));
        }
        let completion = self.root.normal(value)?;
        self.select_root_completion(&completion)?;
        Ok(completion)
    }

    /// Selects root typed failure without appending an instruction or occurrence.
    pub fn fail(&mut self, value: &Value<Failure>) -> Result<BlockCompletion<Output, Failure>>
    where
        Failure: MfmValue,
    {
        self.root.require_visible(&value.slot)?;
        let expected = Failure::failure_contract()?.contract_ref()?;
        if value.slot.contract_ref != expected {
            return Err(ProgramError::Authoring(
                "root failure contract differs from the operation contract".to_owned(),
            ));
        }
        let completion = BlockCompletion::scope_failure(value);
        self.select_root_completion(&completion)?;
        Ok(completion)
    }

    fn select_root_completion(
        &mut self,
        completion: &BlockCompletion<Output, Failure>,
    ) -> Result<()> {
        if self.root_completion.is_some() {
            return Err(ProgramError::Authoring(
                "operation root completion was already selected".to_owned(),
            ));
        }
        self.root_completion = Some(completion.tail.clone());
        Ok(())
    }

    /// Finishes the canonical authored program.
    pub fn finish(
        self,
        completion: BlockCompletion<Output, Failure>,
    ) -> Result<AuthoredStructuredProgram> {
        if self.root_completion.as_ref() != Some(&completion.tail) {
            return Err(ProgramError::Authoring(
                "root completion does not match this builder's affine selection".to_owned(),
            ));
        }
        let failure_contract = Failure::failure_contract()?;
        let root = self.root.into_block(completion)?;
        Ok(AuthoredStructuredProgram {
            operation_id: self.operation_id,
            input_roots: self.input_roots,
            output_contract_ref: self.output_contract_ref,
            failure_contract,
            root,
        })
    }
}

fn validate_pure_never_handler(
    contract: &StructuredStateContract,
    source_contract_ref: &ContentRef,
) -> Result<()> {
    if !matches!(contract.execution, StructuredStateExecutionContract::Pure)
        || !matches!(contract.failure_contract, StructuredFailureContract::Never)
        || &contract.input_contract_ref != source_contract_ref
    {
        return Err(ProgramError::Authoring(
            "failure mapper/handler must be Pure + Never over the exact source contract".to_owned(),
        ));
    }
    Ok(())
}

fn validate_default_route<ScopeFailure: MfmValue>(
    route: &ClosedSumContract,
    handler_output_contract_ref: &ContentRef,
) -> Result<()> {
    route.validate()?;
    let target = ScopeFailure::failure_contract()?.contract_ref()?;
    if &route.selector_contract_ref != handler_output_contract_ref
        || route.variants.len() != 1
        || route.variants[0].canonical_tag != "propagate"
        || route.variants[0].payloads.len() != 1
        || route.variants[0].payloads[0].contract_ref != target
    {
        return Err(ProgramError::Authoring(
            "default handler must return exact Propagate(scope_failure)".to_owned(),
        ));
    }
    Ok(())
}

fn validate_route_arms(route: &ClosedSumContract, arms: &[AuthoredMatchArm]) -> Result<()> {
    let expected: Vec<&str> = route
        .variants
        .iter()
        .map(|variant| variant.canonical_tag.as_str())
        .collect();
    let actual: Vec<&str> = arms.iter().map(|arm| arm.canonical_tag.as_str()).collect();
    if expected != actual {
        return Err(ProgramError::Authoring(
            "custom failure-handler routes are not exhaustive in canonical tag order".to_owned(),
        ));
    }
    Ok(())
}

fn count_policy_proceeds(block: &AuthoredBlock) -> Result<usize> {
    let proceed_ref = policy_proceed_program_ref()?;
    let mut count = 0usize;
    for declaration in &block.declarations {
        match declaration {
            AuthoredDeclaration::State(_) => {}
            AuthoredDeclaration::OperationCall(call) => {
                count = count
                    .checked_add(usize::from(call.child_program_ref == proceed_ref))
                    .ok_or_else(|| {
                        ProgramError::Authoring(
                            "policy proceed declaration count overflowed".to_owned(),
                        )
                    })?;
            }
            AuthoredDeclaration::Match(binding) => {
                for arm in &binding.arms {
                    count = count
                        .checked_add(count_policy_proceeds(&arm.body)?)
                        .ok_or_else(|| {
                            ProgramError::Authoring(
                                "policy proceed declaration count overflowed".to_owned(),
                            )
                        })?;
                }
            }
            AuthoredDeclaration::FanOut(group) => {
                for lane in &group.lanes {
                    count = count
                        .checked_add(count_policy_proceeds(&lane.body)?)
                        .ok_or_else(|| {
                            ProgramError::Authoring(
                                "policy proceed declaration count overflowed".to_owned(),
                            )
                        })?;
                }
            }
        }
    }
    Ok(count)
}

fn lexically_visible(slot: &LexicalSlot, block_path: &StructuralPath, next_ordinal: u32) -> bool {
    let producer = slot.lexical_path.segments();
    let consumer = block_path.segments();
    if producer.first() != consumer.first() {
        return false;
    }
    if producer == consumer {
        return true;
    }
    if producer.len() == consumer.len() + 1 && producer[..consumer.len()] == *consumer {
        return declaration_export_is_visible(slot, &producer[consumer.len()], next_ordinal);
    }
    if producer.len() < consumer.len()
        && producer
            .iter()
            .zip(consumer)
            .all(|(left, right)| left == right)
    {
        return matches!(
            slot.producer,
            LexicalProducer::AdmissionRoot { .. } | LexicalProducer::VariantPayload { .. }
        );
    }
    ordered_sibling_dominates(slot, producer, consumer)
}

fn declaration_export_is_visible(
    slot: &LexicalSlot,
    segment: &StructuralPathSegment,
    next_ordinal: u32,
) -> bool {
    matches!(
        segment,
        StructuralPathSegment::Declaration { ordinal, .. } if *ordinal < next_ordinal
    ) && matches!(
        slot.producer,
        LexicalProducer::AuthoredCallOutput { .. }
            | LexicalProducer::MatchMerge { .. }
            | LexicalProducer::FanOutJoin { .. }
    )
}

fn ordered_sibling_dominates(
    slot: &LexicalSlot,
    producer: &[StructuralPathSegment],
    consumer: &[StructuralPathSegment],
) -> bool {
    if producer.is_empty() || consumer.is_empty() {
        return false;
    }
    let common = producer
        .iter()
        .zip(consumer)
        .take_while(|(left, right)| left == right)
        .count();
    if common >= producer.len() || common >= consumer.len() {
        return false;
    }
    producer.len() == common + 1
        && matches!(
            (&producer[common], &consumer[common]),
            (
                StructuralPathSegment::Declaration { ordinal: left, .. },
                StructuralPathSegment::Declaration { ordinal: right, .. },
            ) if left < right
        )
        && matches!(
            slot.producer,
            LexicalProducer::AuthoredCallOutput { .. }
                | LexicalProducer::MatchMerge { .. }
                | LexicalProducer::FanOutJoin { .. }
        )
}
