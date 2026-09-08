use super::*;
use mfm_capabilities::{CapabilityError, EffectCapabilityContract};
use mfm_program::{
    CapabilityInjection, EffectState, Never, Operation, OperationExpansion, PreparationError,
    ProgramError, ProposedStateOutcome, PureState, State, StateExecutionError,
};
use mfm_values::ContextSlot;
use std::marker::PhantomData;

/// Exact nonce domain, excluding endpoint and custody-provider dimensions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "nonce-domain",
    version = "1",
    schema = "mfm.evm-nonce-domain"
)]
pub struct NonceDomain {
    /// Authority epoch.
    pub authority_epoch: EvmAuthorityEpoch,
    /// Chain instance.
    pub chain_instance: EvmChainInstance,
    /// Sender.
    pub sender: EvmAddress,
}

/// Immutable nonce reservation for one Effect and command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "nonce-reservation",
    version = "1",
    schema = "mfm.evm-nonce-reservation"
)]
pub struct Reservation {
    effect_id: EffectId,
    command_value_ref: ContentRef,
    domain: NonceDomain,
    nonce: u64,
}

impl Reservation {
    /// Constructs one checked reservation from checked public identities.
    pub fn new(
        effect_id: EffectId,
        command_value_ref: ContentRef,
        domain: NonceDomain,
        nonce: u64,
    ) -> Result<Self, EvmDomainError> {
        let reservation = Self {
            effect_id,
            command_value_ref,
            domain,
            nonce,
        };
        reservation.validate()?;
        Ok(reservation)
    }

    /// Returns the reserved Effect identity.
    pub const fn effect_id(&self) -> &EffectId {
        &self.effect_id
    }

    /// Returns the original nonce-free command reference.
    pub const fn command_value_ref(&self) -> &ContentRef {
        &self.command_value_ref
    }

    /// Returns the complete nonce domain.
    pub const fn domain(&self) -> &NonceDomain {
        &self.domain
    }

    /// Returns the reserved nonce.
    pub const fn nonce(&self) -> u64 {
        self.nonce
    }
}

impl NonceDomain {
    /// Derives the endpoint-independent reservation domain.
    pub fn from_binding(binding: &EvmTransactionBinding) -> Self {
        Self {
            authority_epoch: binding.authority_epoch.clone(),
            chain_instance: binding.route.chain_instance.clone(),
            sender: binding.sender.clone(),
        }
    }
}

impl Reservation {
    fn validate(&self) -> Result<(), EvmDomainError> {
        if self.nonce == u64::MAX {
            Err(EvmDomainError::InvalidValue)
        } else {
            Ok(())
        }
    }
}
impl_checked_deserialize!(Reservation {
    effect_id: EffectId,
    command_value_ref: ContentRef,
    domain: NonceDomain,
    nonce: u64
});

/// Public descriptor for the reserved-transaction stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "reserved-transaction",
    version = "1",
    schema = "mfm.evm-reserved-transaction"
)]
pub struct ReservedEvmTransaction {
    command: Eip1559TransactionCommand,
    reservation: Reservation,
}
impl ReservedEvmTransaction {
    /// Constructs and checks the public stage descriptor.
    pub fn new(
        command: Eip1559TransactionCommand,
        reservation: Reservation,
    ) -> Result<Self, EvmDomainError> {
        let value = Self {
            command,
            reservation,
        };
        value.validate()?;
        Ok(value)
    }
    /// Returns the command.
    pub const fn command(&self) -> &Eip1559TransactionCommand {
        &self.command
    }
    /// Returns the reservation.
    pub const fn reservation(&self) -> &Reservation {
        &self.reservation
    }
    fn validate(&self) -> Result<(), EvmDomainError> {
        let (_, reference) =
            canonicalize_mfm_value(&self.command).map_err(|_| EvmDomainError::Program)?;
        if self.reservation.command_value_ref() != &reference
            || self.reservation.domain() != &NonceDomain::from_binding(self.command.binding())
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}
impl_checked_deserialize!(ReservedEvmTransaction {
    command: Eip1559TransactionCommand,
    reservation: Reservation
});

/// Public descriptor for the prepared-transaction stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "prepared-transaction",
    version = "1",
    schema = "mfm.evm-prepared-transaction"
)]
pub struct PreparedEvmTransaction {
    reserved: ReservedEvmTransaction,
    transaction_hash: EvmHash,
}
impl PreparedEvmTransaction {
    /// Combines a checked reservation descriptor with its public retained-wire hash.
    pub const fn new(reserved: ReservedEvmTransaction, transaction_hash: EvmHash) -> Self {
        Self {
            reserved,
            transaction_hash,
        }
    }
    /// Returns the reserved transaction.
    pub const fn reserved(&self) -> &ReservedEvmTransaction {
        &self.reserved
    }
    /// Returns the transaction hash.
    pub const fn transaction_hash(&self) -> &EvmHash {
        &self.transaction_hash
    }
}

/// Evidence identifying the immutable prepared wire for one preparation Effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-preparation-evidence",
    version = "1",
    schema = "mfm.evm-transaction-preparation-evidence"
)]
pub struct PreparedEvmTransactionEvidence {
    /// Effect id.
    pub effect_id: EffectId,
    /// Transaction hash.
    pub transaction_hash: EvmHash,
}

pub(super) fn action_matches(
    command: &Eip1559TransactionCommand,
    evidence: &EvmTransactionSettlement,
) -> bool {
    matches!(
        (command.to(), evidence.outcome()),
        (
            None,
            EvmTransactionOutcome::Created { .. } | EvmTransactionOutcome::Reverted
        ) | (
            Some(_),
            EvmTransactionOutcome::Called | EvmTransactionOutcome::Reverted
        )
    )
}

/// Capability for the reserve-nonce Effect.
pub struct EvmNonceReservationEffect;
impl EffectCapabilityContract for EvmNonceReservationEffect {
    type Command = Eip1559TransactionCommand;
    type Evidence = Reservation;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.evm.capability.reserve-nonce@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        effect_id: &EffectId,
        command: &Self::Command,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        let matches = {
            let (_, reference) =
                canonicalize_mfm_value(command).map_err(|_| CapabilityError::EvidenceBinding)?;
            evidence.effect_id() == effect_id
                && evidence.command_value_ref() == &reference
                && evidence.domain() == &NonceDomain::from_binding(command.binding())
        };
        matches
            .then_some(())
            .ok_or(CapabilityError::EvidenceBinding)
    }
}

/// Capability for the prepare-transaction Effect.
pub struct EvmTransactionPreparationEffect;
impl EffectCapabilityContract for EvmTransactionPreparationEffect {
    type Command = ReservedEvmTransaction;
    type Evidence = PreparedEvmTransactionEvidence;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.evm.capability.prepare-transaction@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        effect_id: &EffectId,
        command: &Self::Command,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        let matches = {
            let _ = command;
            (&evidence.effect_id) == effect_id
        };
        matches
            .then_some(())
            .ok_or(CapabilityError::EvidenceBinding)
    }
}

/// Capability for the execute-transaction Effect.
pub struct EvmTransactionEffect;
impl EffectCapabilityContract for EvmTransactionEffect {
    type Command = PreparedEvmTransaction;
    type Evidence = EvmTransactionSettlement;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new(EVM_TRANSACTION_EFFECT_CAPABILITY_ID)
            .map_err(|_| CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        effect_id: &EffectId,
        command: &Self::Command,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        let matches = {
            evidence.effect_id() == effect_id
                && evidence.nonce() == command.reserved().reservation().nonce()
                && evidence.transaction_hash() == command.transaction_hash()
                && action_matches(command.reserved().command(), evidence)
        };
        matches
            .then_some(())
            .ok_or(CapabilityError::EvidenceBinding)
    }
}

/// Context after replacing the recipe's selected field with an exact fact type.
pub type Replaced<C, R, V> = <<R as TransactionRecipe<C>>::Slot as ContextSlot<C>>::With<V>;
/// Complete context after nonce reservation.
pub type ReservedContext<C, R> = Replaced<C, R, ReservedEvmTransaction>;
/// Complete context after transaction preparation.
pub type PreparedContext<C, R> = Replaced<C, R, PreparedTransactionFacts>;
/// Complete context after authenticated settlement.
pub type ExecutedContext<C, R> = Replaced<C, R, ExecutedTransactionFacts>;
/// Complete context after successful outcome projection.
pub type CompletedContext<C, R> =
    Replaced<C, R, CompletedTransactionFacts<<R as TransactionRecipe<C>>::Success>>;

fn state_id<C: MfmValueTrait, R: TransactionRecipe<C>>(
    stage: &str,
) -> mfm_program::Result<StableId> {
    super::recipes::executable_id(
        stage,
        R::recipe_id().map_err(|_| ProgramError::InvalidContract)?,
        R::source_ids().map_err(|_| ProgramError::InvalidContract)?,
        R::Success::mode_id(),
    )
}

/// The reserve-nonce State, retaining all sibling context fields.
pub struct ReserveEvmNonce<C, R>(PhantomData<fn() -> (C, R)>);
impl<C: MfmValueTrait, R: TransactionRecipe<C>> State for ReserveEvmNonce<C, R> {
    type Input = C;
    type Output = ReservedContext<C, R>;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        state_id::<C, R>("reserve-nonce")
    }
}
/// The prepare-transaction State, retaining all sibling context fields.
pub struct PrepareEvmTransaction<C, R>(PhantomData<fn() -> (C, R)>);
impl<C: MfmValueTrait, R: TransactionRecipe<C>> State for PrepareEvmTransaction<C, R> {
    type Input = ReservedContext<C, R>;
    type Output = PreparedContext<C, R>;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        state_id::<C, R>("prepare-transaction")
    }
}
/// The execute-transaction State, retaining all sibling context fields.
pub struct ExecuteEvmTransaction<C, R>(PhantomData<fn() -> (C, R)>);
impl<C: MfmValueTrait, R: TransactionRecipe<C>> State for ExecuteEvmTransaction<C, R> {
    type Input = PreparedContext<C, R>;
    type Output = ExecutedContext<C, R>;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        state_id::<C, R>("execute-transaction")
    }
}
/// The project-transaction-outcome State, retaining all sibling context fields.
pub struct ProjectEvmTransactionOutcome<C, R>(PhantomData<fn() -> (C, R)>);
impl<C: MfmValueTrait, R: TransactionRecipe<C>> State for ProjectEvmTransactionOutcome<C, R> {
    type Input = ExecutedContext<C, R>;
    type Output = CompletedContext<C, R>;
    type Failure = EvmTransactionFailure<ExecutedContext<C, R>>;
    fn state_id() -> mfm_program::Result<StableId> {
        state_id::<C, R>("project-transaction-outcome")
    }
}

impl<C: MfmValueTrait, R: TransactionRecipe<C>> EffectState<EvmNonceReservationEffect>
    for ReserveEvmNonce<C, R>
{
    fn prepare(input: &C) -> Result<Eip1559TransactionCommand, PreparationError> {
        let command = R::command(input);
        if !R::Success::accepts(&command) {
            return Err(PreparationError);
        }
        Ok(command)
    }
    fn interpret(
        input: C,
        evidence: &Reservation,
    ) -> Result<ProposedStateOutcome<Self::Output, Never>, StateExecutionError> {
        let command = R::command(&input);
        if !R::Success::accepts(&command) {
            return Err(StateExecutionError);
        }
        let facts = ReservedEvmTransaction::new(command, evidence.clone())
            .map_err(|_| StateExecutionError)?;
        Ok(ProposedStateOutcome::Success {
            output: <R::Slot as ContextSlot<C>>::replace(input, facts),
        })
    }
}
impl<C: MfmValueTrait, R: TransactionRecipe<C>> EffectState<EvmTransactionPreparationEffect>
    for PrepareEvmTransaction<C, R>
where
    R::Slot: ContextSlot<
        ReservedContext<C, R>,
        Value = ReservedEvmTransaction,
        With<PreparedTransactionFacts> = PreparedContext<C, R>,
    >,
{
    fn prepare(input: &Self::Input) -> Result<ReservedEvmTransaction, PreparationError> {
        Ok(<R::Slot as ContextSlot<Self::Input>>::get(input).clone())
    }
    fn interpret(
        input: Self::Input,
        evidence: &PreparedEvmTransactionEvidence,
    ) -> Result<ProposedStateOutcome<Self::Output, Never>, StateExecutionError> {
        let facts = PreparedTransactionFacts::new(
            <R::Slot as ContextSlot<Self::Input>>::get(&input).clone(),
            evidence.clone(),
        );
        Ok(ProposedStateOutcome::Success {
            output: <R::Slot as ContextSlot<Self::Input>>::replace(input, facts),
        })
    }
}
impl<C: MfmValueTrait, R: TransactionRecipe<C>> EffectState<EvmTransactionEffect>
    for ExecuteEvmTransaction<C, R>
where
    R::Slot: ContextSlot<
        PreparedContext<C, R>,
        Value = PreparedTransactionFacts,
        With<ExecutedTransactionFacts> = ExecutedContext<C, R>,
    >,
{
    fn prepare(input: &Self::Input) -> Result<PreparedEvmTransaction, PreparationError> {
        Ok(<R::Slot as ContextSlot<Self::Input>>::get(input).execution_command())
    }
    fn interpret(
        input: Self::Input,
        evidence: &EvmTransactionSettlement,
    ) -> Result<ProposedStateOutcome<Self::Output, Never>, StateExecutionError> {
        let facts = ExecutedTransactionFacts::new(
            <R::Slot as ContextSlot<Self::Input>>::get(&input).clone(),
            evidence.clone(),
        )
        .map_err(|_| StateExecutionError)?;
        Ok(ProposedStateOutcome::Success {
            output: <R::Slot as ContextSlot<Self::Input>>::replace(input, facts),
        })
    }
}
impl<C: MfmValueTrait, R: TransactionRecipe<C>> PureState for ProjectEvmTransactionOutcome<C, R>
where
    R::Slot: ContextSlot<
        ExecutedContext<C, R>,
        Value = ExecutedTransactionFacts,
        With<CompletedTransactionFacts<R::Success>> = CompletedContext<C, R>,
    >,
{
    fn evaluate(
        input: Self::Input,
    ) -> Result<ProposedStateOutcome<Self::Output, Self::Failure>, StateExecutionError> {
        let facts = <R::Slot as ContextSlot<Self::Input>>::get(&input);
        if !R::Success::accepts(facts.command()) {
            return Err(StateExecutionError);
        }
        if matches!(
            facts.settlement().outcome(),
            EvmTransactionOutcome::Reverted
        ) {
            return Ok(ProposedStateOutcome::Failure {
                failure: EvmTransactionFailure::new(input),
            });
        }
        let completed = CompletedTransactionFacts::<R::Success>::new(facts.clone())?;
        Ok(ProposedStateOutcome::Success {
            output: <R::Slot as ContextSlot<Self::Input>>::replace(input, completed),
        })
    }
}

impl<C: MfmValueTrait, R: TransactionRecipe<C>> CapabilityInjection<ReserveEvmNonce<C, R>>
    for EvmNonceReservationEffect
{
    type Setup = EvmTransactionBinding;
    type ExpandedInput = <ReserveEvmNonce<C, R> as State>::Input;
    type ExpandedOutput = <ReserveEvmNonce<C, R> as State>::Output;
    type ExpandedFailure = Never;
    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        setup
            .binding_ref()
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl<C: MfmValueTrait, R: TransactionRecipe<C>> CapabilityInjection<PrepareEvmTransaction<C, R>>
    for EvmTransactionPreparationEffect
{
    type Setup = EvmTransactionBinding;
    type ExpandedInput = <PrepareEvmTransaction<C, R> as State>::Input;
    type ExpandedOutput = <PrepareEvmTransaction<C, R> as State>::Output;
    type ExpandedFailure = Never;
    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        setup
            .binding_ref()
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl<C: MfmValueTrait, R: TransactionRecipe<C>> CapabilityInjection<ExecuteEvmTransaction<C, R>>
    for EvmTransactionEffect
where
    PrepareEvmTransaction<C, R>: EffectState<EvmTransactionPreparationEffect>,
    ProjectEvmTransactionOutcome<C, R>: PureState,
{
    type Setup = EvmTransactionBinding;
    type ExpandedInput = C;
    type ExpandedOutput = CompletedContext<C, R>;
    type ExpandedFailure = EvmTransactionFailure<ExecutedContext<C, R>>;
    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        setup
            .binding_ref()
            .map_err(|_| ProgramError::InvalidContract)
    }
    fn write_before(
        setup: &Self::Setup,
        expansion: &mut OperationExpansion<
            Self::ExpandedInput,
            <ExecuteEvmTransaction<C, R> as State>::Input,
            Self::ExpandedFailure,
        >,
    ) -> mfm_program::Result<()> {
        expansion.effect::<ReserveEvmNonce<C, R>, EvmNonceReservationEffect>(setup)?;
        expansion.effect::<PrepareEvmTransaction<C, R>, EvmTransactionPreparationEffect>(setup)
    }
    fn write_after(
        _: &Self::Setup,
        expansion: &mut OperationExpansion<
            <ExecuteEvmTransaction<C, R> as State>::Output,
            Self::ExpandedOutput,
            Self::ExpandedFailure,
        >,
    ) -> mfm_program::Result<()> {
        expansion.pure::<ProjectEvmTransactionOutcome<C, R>>()
    }
}

/// Public one-transaction authoring entry point; injection installs all four durable States.
pub struct EvmTransaction<C, R> {
    binding: EvmTransactionBinding,
    context: PhantomData<fn() -> (C, R)>,
}
impl<C, R> EvmTransaction<C, R> {
    /// Selects the explicit adapter binding for this authored transaction.
    pub const fn new(binding: EvmTransactionBinding) -> Self {
        Self {
            binding,
            context: PhantomData,
        }
    }
}
impl<C: MfmValueTrait, R: TransactionRecipe<C>> Operation for EvmTransaction<C, R>
where
    ExecuteEvmTransaction<C, R>: EffectState<EvmTransactionEffect>,
    EvmTransactionEffect: CapabilityInjection<
        ExecuteEvmTransaction<C, R>,
        Setup = EvmTransactionBinding,
        ExpandedInput = C,
        ExpandedOutput = CompletedContext<C, R>,
        ExpandedFailure = EvmTransactionFailure<ExecutedContext<C, R>>,
    >,
{
    type Input = C;
    type Output = CompletedContext<C, R>;
    type Failure = EvmTransactionFailure<ExecutedContext<C, R>>;
    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.effect::<ExecuteEvmTransaction<C, R>, EvmTransactionEffect>(&self.binding)
    }
}
