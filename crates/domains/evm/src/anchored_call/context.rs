use super::*;
use crate::{Called, CompletedTransactionFacts};
use mfm_program::{
    CapabilityInjection, PreparationError, ProgramError, ProposedStateOutcome, ReadState, State,
    StateExecutionError,
};
use mfm_values::{canonicalize_mfm_value, ContextSlot, MfmValue as MfmValueTrait};
use std::marker::PhantomData;

/// Checked route and bounded calldata awaiting a transaction target and receipt anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "checked-observation-plan",
    version = "1",
    schema = "mfm.evm-checked-observation-plan"
)]
pub struct CheckedObservationPlan {
    chain_id: NonZeroU64,
    route_ref: ContentRef,
    #[mfm(minimum_bytes = 0, maximum_bytes = 131072)]
    calldata: CanonicalBytes,
}
impl CheckedObservationPlan {
    /// Checks independent observation inputs and derives the route reference before admission.
    pub fn new(route: EvmTransactionRoute, calldata: Vec<u8>) -> Result<Self, EvmDomainError> {
        crate::transaction::validate_input_bytes(&calldata, MAX_EVM_CALLDATA_BYTES)?;
        Ok(Self {
            chain_id: route.chain_instance.chain_id,
            route_ref: route.binding_ref()?,
            calldata: CanonicalBytes::new(calldata),
        })
    }
    /// Returns the selected chain.
    pub const fn chain_id(&self) -> NonZeroU64 {
        self.chain_id
    }
    /// Returns the checked route reference.
    pub const fn route_ref(&self) -> &ContentRef {
        &self.route_ref
    }
    /// Constructs the exact intent after checked target and anchor facts become available.
    pub fn intent_for(
        &self,
        target: EvmAddress,
        anchor: EvmBlockAnchor,
    ) -> AnchoredContractCallIntent {
        AnchoredContractCallIntent {
            chain_id: self.chain_id,
            route_ref: self.route_ref.clone(),
            calldata: self.calldata.clone(),
            target,
            anchor,
        }
    }
}
impl<'de> Deserialize<'de> for CheckedObservationPlan {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            chain_id: NonZeroU64,
            route_ref: ContentRef,
            calldata: CanonicalBytes,
        }
        let wire = Wire::deserialize(deserializer)?;
        crate::transaction::validate_input_bytes(wire.calldata.as_bytes(), MAX_EVM_CALLDATA_BYTES)
            .map_err(de::Error::custom)?;
        Ok(Self {
            chain_id: wire.chain_id,
            route_ref: wire.route_ref,
            calldata: wire.calldata,
        })
    }
}

/// Exact intent and accepted anchored evidence, including reviewed domain failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchored-observation-facts",
    version = "1",
    schema = "mfm.evm-anchored-observation-facts"
)]
pub struct AnchoredObservationFacts {
    intent: AnchoredContractCallIntent,
    evidence: AnchoredContractCallEvidence,
}
impl AnchoredObservationFacts {
    /// Checks the exact intent reference and anchor relationship of accepted evidence.
    pub fn new(
        intent: AnchoredContractCallIntent,
        evidence: AnchoredContractCallEvidence,
    ) -> Result<Self, EvmDomainError> {
        let (_, reference) =
            canonicalize_mfm_value(&intent).map_err(|_| EvmDomainError::Program)?;
        EvmAnchoredContractCallRead::bind_evidence(&reference, &intent, &evidence)
            .map_err(|_| EvmDomainError::EvidenceBinding)?;
        Ok(Self { intent, evidence })
    }
    /// Returns the exact retained intent.
    pub const fn intent(&self) -> &AnchoredContractCallIntent {
        &self.intent
    }
    /// Returns all accepted evidence without discarding failure facts.
    pub const fn evidence(&self) -> &AnchoredContractCallEvidence {
        &self.evidence
    }
    /// Returns the raw returned result when this observation succeeded.
    pub const fn result(&self) -> Option<&AnchoredContractCallResult> {
        match &self.evidence {
            AnchoredContractCallEvidence::Returned { result, .. } => Some(result),
            _ => None,
        }
    }
    /// Returns the reviewed failure reason when the evidence is a domain failure.
    pub const fn failure_reason(&self) -> Option<AnchoredContractCallFailureReason> {
        match &self.evidence {
            AnchoredContractCallEvidence::Returned { .. } => None,
            AnchoredContractCallEvidence::Rejected { .. } => {
                Some(AnchoredContractCallFailureReason::Rejected)
            }
            AnchoredContractCallEvidence::SafeFailure { .. } => {
                Some(AnchoredContractCallFailureReason::SafeFailure)
            }
            AnchoredContractCallEvidence::IntegrityBlocked { .. } => {
                Some(AnchoredContractCallFailureReason::IntegrityBlocked)
            }
        }
    }
}
impl<'de> Deserialize<'de> for AnchoredObservationFacts {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            intent: AnchoredContractCallIntent,
            evidence: AnchoredContractCallEvidence,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.intent, wire.evidence).map_err(de::Error::custom)
    }
}

/// Deterministic policy for selecting the destination field and exact anchored intent.
pub trait ObservationRecipe<C: MfmValueTrait>: Send + Sync + 'static {
    /// Destination field replaced by accepted observation facts.
    type Slot: ContextSlot<C>;
    /// Stable intent-selection implementation identity.
    fn recipe_id() -> mfm_values::Result<StableId>;
    /// Ordered destination then source slot identities.
    fn source_ids() -> mfm_values::Result<Vec<StableId>>;
    /// Constructs the intent, rejecting local cross-field mismatches before provider entry.
    fn intent(context: &C) -> Result<AnchoredContractCallIntent, PreparationError>;
}

/// Observes a completed ordinary call at its retained target and exact receipt anchor.
pub struct ObserveAt<O, T>(PhantomData<fn() -> (O, T)>);
impl<C, O, T> ObservationRecipe<C> for ObserveAt<O, T>
where
    C: MfmValueTrait,
    O: ContextSlot<C, Value = CheckedObservationPlan> + 'static,
    T: ContextSlot<C, Value = CompletedTransactionFacts<Called>> + 'static,
{
    type Slot = O;
    fn recipe_id() -> mfm_values::Result<StableId> {
        crate::transaction::recipes::recipe_id("mfm.evm.recipe.observe-at@1")
    }
    fn source_ids() -> mfm_values::Result<Vec<StableId>> {
        Ok(vec![O::slot_id()?, T::slot_id()?])
    }
    fn intent(context: &C) -> Result<AnchoredContractCallIntent, PreparationError> {
        let plan = O::get(context);
        let call = T::get(context);
        let route = &call.command().binding().route;
        let reference = route.binding_ref().map_err(|_| PreparationError)?;
        if &reference != plan.route_ref() || route.chain_instance.chain_id != plan.chain_id() {
            return Err(PreparationError);
        }
        Ok(plan.intent_for(
            call.outcome().target().clone(),
            call.settlement().block_anchor().clone(),
        ))
    }
}

/// Complete context after retaining accepted anchored observation evidence.
pub type ObservedContext<C, R> =
    <<R as ObservationRecipe<C>>::Slot as ContextSlot<C>>::With<AnchoredObservationFacts>;

/// Accumulated evidence context and reviewed reason at an anchored Read's failure branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "C: serde::de::DeserializeOwned")
)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchored-contract-call-failure",
    version = "2",
    schema = "mfm.evm-anchored-contract-call-failure"
)]
pub struct AnchoredContractCallFailure<C: MfmValueTrait> {
    context: C,
    reason: AnchoredContractCallFailureReason,
}
impl<C: MfmValueTrait> AnchoredContractCallFailure<C> {
    /// Returns the complete accumulated context including accepted failure evidence.
    pub const fn context(&self) -> &C {
        &self.context
    }
    /// Returns the reviewed domain failure reason.
    pub const fn reason(&self) -> AnchoredContractCallFailureReason {
        self.reason
    }
    /// Moves the context into the product's failure report policy.
    pub fn into_context(self) -> C {
        self.context
    }
}

/// Anchored Read whose prepare callback constructs intent and whose conclusion accumulates evidence.
pub struct ReadAnchoredContractCall<C, R>(PhantomData<fn() -> (C, R)>);
impl<C: MfmValueTrait, R: ObservationRecipe<C>> State for ReadAnchoredContractCall<C, R> {
    type Input = C;
    type Output = ObservedContext<C, R>;
    type Failure = AnchoredContractCallFailure<ObservedContext<C, R>>;
    fn state_id() -> mfm_program::Result<StableId> {
        crate::transaction::recipes::executable_id(
            "read-anchored-contract-call",
            R::recipe_id().map_err(|_| ProgramError::InvalidContract)?,
            R::source_ids().map_err(|_| ProgramError::InvalidContract)?,
            "observation",
        )
    }
}
impl<C: MfmValueTrait, R: ObservationRecipe<C>> ReadState<EvmAnchoredContractCallRead>
    for ReadAnchoredContractCall<C, R>
{
    type AdapterContext = crate::AnchoredCallAdapterContext;
    fn adapter_context(
        input: &C,
        intent: &AnchoredContractCallIntent,
        _: &crate::EvmOperationalError,
    ) -> Result<Self::AdapterContext, StateExecutionError> {
        if &R::intent(input).map_err(|_| StateExecutionError)? != intent {
            return Err(StateExecutionError);
        }
        Ok(crate::AnchoredCallAdapterContext::from_intent(intent))
    }
    fn prepare(input: &C) -> Result<AnchoredContractCallIntent, PreparationError> {
        R::intent(input)
    }
    fn interpret(
        input: C,
        evidence: &AnchoredContractCallEvidence,
    ) -> Result<ProposedStateOutcome<Self::Output, Self::Failure>, StateExecutionError> {
        let intent = R::intent(&input).map_err(|_| StateExecutionError)?;
        let facts = AnchoredObservationFacts::new(intent, evidence.clone())
            .map_err(|_| StateExecutionError)?;
        let reason = facts.failure_reason();
        let context = R::Slot::replace(input, facts);
        Ok(match reason {
            None => ProposedStateOutcome::Success { output: context },
            Some(reason) => ProposedStateOutcome::Failure {
                failure: AnchoredContractCallFailure { context, reason },
            },
        })
    }
}
impl<C: MfmValueTrait, R: ObservationRecipe<C>> CapabilityInjection<ReadAnchoredContractCall<C, R>>
    for EvmAnchoredContractCallRead
{
    type Setup = EvmTransactionRoute;
    type ExpandedInput = C;
    type ExpandedOutput = ObservedContext<C, R>;
    type ExpandedFailure = AnchoredContractCallFailure<ObservedContext<C, R>>;
    type FailureMap = mfm_program::Identity<Self::ExpandedFailure>;
    fn failure_map_params(_: &Self::Setup) -> mfm_program::Result<mfm_program::NoParams> {
        Ok(mfm_program::NoParams)
    }
    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        setup
            .binding_ref()
            .map_err(|_| ProgramError::InvalidContract)
    }
}
