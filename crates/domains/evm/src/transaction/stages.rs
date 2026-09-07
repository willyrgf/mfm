use super::*;
use mfm_capabilities::{CapabilityError, EffectCapabilityContract};
use mfm_program::{
    CapabilityInjection, EffectState, Never, OperationExpansion, PreparationError, ProgramError,
    ProposedStateOutcome, PureState, State,
};
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
    authority_epoch: EvmAuthorityEpoch,
    chain_instance: EvmChainInstance,
    sender: EvmAddress,
}

impl NonceDomain {
    /// Constructs a nonce domain from checked public identities.
    pub fn new(
        authority_epoch: EvmAuthorityEpoch,
        chain_instance: EvmChainInstance,
        sender: EvmAddress,
    ) -> Self {
        Self {
            authority_epoch,
            chain_instance,
            sender,
        }
    }

    /// Returns the authority epoch that separates fresh installations.
    pub const fn authority_epoch(&self) -> &EvmAuthorityEpoch {
        &self.authority_epoch
    }

    /// Returns the endpoint-independent chain instance.
    pub const fn chain_instance(&self) -> &EvmChainInstance {
        &self.chain_instance
    }

    /// Returns the exact sender address.
    pub const fn sender(&self) -> &EvmAddress {
        &self.sender
    }
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
        if nonce == u64::MAX {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self {
            effect_id,
            command_value_ref,
            domain,
            nonce,
        })
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
        Self::new(
            binding.authority_epoch().clone(),
            binding.route().chain_instance().clone(),
            binding.sender().clone(),
        )
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
checked_deserialize!(Reservation {
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
checked_deserialize!(ReservedEvmTransaction {
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

/// Public descriptor for the executed-transaction stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "executed-transaction",
    version = "1",
    schema = "mfm.evm-executed-transaction"
)]
pub struct ExecutedEvmTransaction {
    command: Eip1559TransactionCommand,
    settlement: EvmTransactionSettlement,
}
impl ExecutedEvmTransaction {
    /// Constructs and checks the public stage descriptor.
    pub fn new(
        command: Eip1559TransactionCommand,
        settlement: EvmTransactionSettlement,
    ) -> Result<Self, EvmDomainError> {
        let value = Self {
            command,
            settlement,
        };
        value.validate()?;
        Ok(value)
    }
    /// Returns the command.
    pub const fn command(&self) -> &Eip1559TransactionCommand {
        &self.command
    }
    /// Returns the settlement.
    pub const fn settlement(&self) -> &EvmTransactionSettlement {
        &self.settlement
    }
    fn validate(&self) -> Result<(), EvmDomainError> {
        if !action_matches(&self.command, &self.settlement) {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}
checked_deserialize!(ExecutedEvmTransaction {
    command: Eip1559TransactionCommand,
    settlement: EvmTransactionSettlement
});

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
    effect_id: EffectId,
    transaction_hash: EvmHash,
}
impl PreparedEvmTransactionEvidence {
    /// Constructs the public preparation evidence.
    pub const fn new(effect_id: EffectId, transaction_hash: EvmHash) -> Self {
        Self {
            effect_id,
            transaction_hash,
        }
    }
    /// Returns the preparation Effect identity.
    pub const fn effect_id(&self) -> &EffectId {
        &self.effect_id
    }
    /// Returns the retained wire hash.
    pub const fn transaction_hash(&self) -> &EvmHash {
        &self.transaction_hash
    }
}
fn action_matches(
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
            evidence.effect_id() == effect_id
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

/// Context-preserving reserve-nonce State.
pub struct ReserveEvmNonce<K: MfmValueTrait>(PhantomData<fn() -> K>);
impl<K: MfmValueTrait> State for ReserveEvmNonce<K> {
    type Input = EvmTransactionContext<K, Eip1559TransactionCommand>;
    type Output = EvmTransactionContext<K, ReservedEvmTransaction>;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.evm.state.reserve-nonce@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl<K: MfmValueTrait> EffectState<EvmNonceReservationEffect> for ReserveEvmNonce<K> {
    fn prepare(input: &Self::Input) -> Result<Eip1559TransactionCommand, PreparationError> {
        Ok(input.command.clone())
    }
    fn interpret(
        input: Self::Input,
        evidence: &Reservation,
    ) -> std::result::Result<
        ProposedStateOutcome<Self::Output, Self::Failure>,
        mfm_program::StateExecutionError,
    > {
        Ok(ProposedStateOutcome::Success {
            output: EvmTransactionContext::new(
                input.caller_context,
                ReservedEvmTransaction {
                    command: input.command,
                    reservation: evidence.clone(),
                },
            ),
        })
    }
}
impl<K: MfmValueTrait> CapabilityInjection<ReserveEvmNonce<K>> for EvmNonceReservationEffect {
    type Setup = EvmTransactionBinding;
    type ExpandedInput = <ReserveEvmNonce<K> as State>::Input;
    type ExpandedOutput = <ReserveEvmNonce<K> as State>::Output;
    type ExpandedFailure = Never;
    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        setup
            .binding_ref()
            .map_err(|_| ProgramError::InvalidContract)
    }
}

/// Context-preserving prepare-transaction State.
pub struct PrepareEvmTransaction<K: MfmValueTrait>(PhantomData<fn() -> K>);
impl<K: MfmValueTrait> State for PrepareEvmTransaction<K> {
    type Input = EvmTransactionContext<K, ReservedEvmTransaction>;
    type Output = EvmTransactionContext<K, PreparedEvmTransaction>;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.evm.state.prepare-transaction@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}
impl<K: MfmValueTrait> EffectState<EvmTransactionPreparationEffect> for PrepareEvmTransaction<K> {
    fn prepare(input: &Self::Input) -> Result<ReservedEvmTransaction, PreparationError> {
        Ok(input.command.clone())
    }
    fn interpret(
        input: Self::Input,
        evidence: &PreparedEvmTransactionEvidence,
    ) -> std::result::Result<
        ProposedStateOutcome<Self::Output, Self::Failure>,
        mfm_program::StateExecutionError,
    > {
        Ok(ProposedStateOutcome::Success {
            output: EvmTransactionContext::new(
                input.caller_context,
                PreparedEvmTransaction {
                    reserved: input.command,
                    transaction_hash: evidence.transaction_hash.clone(),
                },
            ),
        })
    }
}
impl<K: MfmValueTrait> CapabilityInjection<PrepareEvmTransaction<K>>
    for EvmTransactionPreparationEffect
{
    type Setup = EvmTransactionBinding;
    type ExpandedInput = <PrepareEvmTransaction<K> as State>::Input;
    type ExpandedOutput = <PrepareEvmTransaction<K> as State>::Output;
    type ExpandedFailure = Never;
    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        setup
            .binding_ref()
            .map_err(|_| ProgramError::InvalidContract)
    }
}

/// Context-preserving execute-transaction State.
pub struct ExecuteEvmTransaction<K: MfmValueTrait>(PhantomData<fn() -> K>);
impl<K: MfmValueTrait> State for ExecuteEvmTransaction<K> {
    type Input = EvmTransactionContext<K, PreparedEvmTransaction>;
    type Output = EvmTransactionContext<K, ExecutedEvmTransaction>;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new(EXECUTE_EVM_TRANSACTION_STATE_ID).map_err(|_| ProgramError::InvalidContract)
    }
}
impl<K: MfmValueTrait> EffectState<EvmTransactionEffect> for ExecuteEvmTransaction<K> {
    fn prepare(input: &Self::Input) -> Result<PreparedEvmTransaction, PreparationError> {
        Ok(input.command.clone())
    }
    fn interpret(
        input: Self::Input,
        evidence: &EvmTransactionSettlement,
    ) -> std::result::Result<
        ProposedStateOutcome<Self::Output, Self::Failure>,
        mfm_program::StateExecutionError,
    > {
        Ok(ProposedStateOutcome::Success {
            output: EvmTransactionContext::new(
                input.caller_context,
                ExecutedEvmTransaction {
                    command: input.command.reserved.command,
                    settlement: evidence.clone(),
                },
            ),
        })
    }
}

/// Projects authenticated settlement into typed success or reversion.
pub struct ProjectEvmTransactionOutcome<K: MfmValueTrait>(PhantomData<fn() -> K>);
impl<K: MfmValueTrait> State for ProjectEvmTransactionOutcome<K> {
    type Input = EvmTransactionContext<K, ExecutedEvmTransaction>;
    type Output = EvmTransactionCompletion<K>;
    type Failure = EvmTransactionReversion<K>;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.evm.state.project-transaction-outcome@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}
impl<K: MfmValueTrait> PureState for ProjectEvmTransactionOutcome<K> {
    fn evaluate(
        input: Self::Input,
    ) -> std::result::Result<
        ProposedStateOutcome<Self::Output, Self::Failure>,
        mfm_program::StateExecutionError,
    > {
        let EvmTransactionContext {
            caller_context,
            command,
        } = input;
        let ExecutedEvmTransaction {
            command,
            settlement: evidence,
        } = command;
        let receipt = evidence.receipt().clone();
        let binding = command.binding().clone();
        match (command.to(), evidence.outcome()) {
            (None, EvmTransactionOutcome::Created { created_address }) => {
                Ok(ProposedStateOutcome::Success {
                    output: EvmTransactionCompletion {
                        caller_context,
                        binding,
                        receipt,
                        outcome: EvmTransactionSuccess::Created {
                            created_address: created_address.clone(),
                        },
                    },
                })
            }
            (Some(target), EvmTransactionOutcome::Called) => Ok(ProposedStateOutcome::Success {
                output: EvmTransactionCompletion {
                    caller_context,
                    binding,
                    receipt,
                    outcome: EvmTransactionSuccess::Called {
                        target: target.clone(),
                    },
                },
            }),
            (Some(_), EvmTransactionOutcome::Created { .. })
            | (None, EvmTransactionOutcome::Called) => Err(mfm_program::StateExecutionError),
            (_, EvmTransactionOutcome::Reverted) => Ok(ProposedStateOutcome::Failure {
                failure: EvmTransactionReversion {
                    caller_context,
                    receipt,
                },
            }),
        }
    }
}

impl<K: MfmValueTrait> CapabilityInjection<ExecuteEvmTransaction<K>> for EvmTransactionEffect {
    type Setup = EvmTransactionBinding;
    type ExpandedInput = EvmTransactionContext<K>;
    type ExpandedOutput = EvmTransactionCompletion<K>;
    type ExpandedFailure = EvmTransactionReversion<K>;
    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        setup
            .binding_ref()
            .map_err(|_| ProgramError::InvalidContract)
    }
    fn write_before(
        setup: &Self::Setup,
        expansion: &mut OperationExpansion<
            Self::ExpandedInput,
            <ExecuteEvmTransaction<K> as State>::Input,
            Self::ExpandedFailure,
        >,
    ) -> mfm_program::Result<()> {
        expansion.effect::<ReserveEvmNonce<K>, EvmNonceReservationEffect>(setup)?;
        expansion.effect::<PrepareEvmTransaction<K>, EvmTransactionPreparationEffect>(setup)
    }
    fn write_after(
        _setup: &Self::Setup,
        expansion: &mut OperationExpansion<
            <ExecuteEvmTransaction<K> as State>::Output,
            Self::ExpandedOutput,
            Self::ExpandedFailure,
        >,
    ) -> mfm_program::Result<()> {
        expansion.pure::<ProjectEvmTransactionOutcome<K>>()
    }
}
