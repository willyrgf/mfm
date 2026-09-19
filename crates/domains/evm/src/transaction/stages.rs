use super::*;
use mfm_capabilities::{CapabilityError, EffectCapabilityContract};
use mfm_chain::transaction::{PreparedTransaction, TransactionEffect};
use mfm_program::{EffectSelection, EffectState, Never, ProposedStateOutcome, State};
use mfm_values::{InvocationDiagnostic, Object};
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

/// Rejection while matching a reserved transaction to its complete native command.
#[derive(Debug, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum ReservationBindingError {
    /// Canonical command identity could not be computed; the complete cause is retained.
    #[error("reservation command identity failed: {0}")]
    Identity(#[from] mfm_values::ValueError),
    /// Reservation names a different command.
    #[error("reservation command reference mismatch")]
    Command,
    /// Reservation belongs to another authority, chain or sender domain.
    #[error("reservation nonce domain mismatch")]
    Domain,
}

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
    ) -> Result<Self, ReservationBindingError> {
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
    fn validate(&self) -> Result<(), ReservationBindingError> {
        let (_, reference) = canonicalize_mfm_value(&self.command)?;
        if self.reservation.command_value_ref() != &reference {
            return Err(ReservationBindingError::Command);
        }
        if self.reservation.domain() != &NonceDomain::from_binding(self.command.binding()) {
            return Err(ReservationBindingError::Domain);
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

/// Reservation protocol retaining the native command identity and nonce domain.
pub struct EvmNonceReservationEffect;
impl EffectCapabilityContract for EvmNonceReservationEffect {
    type Command = Eip1559TransactionCommand;
    type Evidence = Reservation;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.evm.capability.reserve-nonce@2")?)
    }
    fn bind_evidence(
        effect_id: &EffectId,
        command_ref: &ContentRef,
        command: &Self::Command,
        _: &ContentRef,
        evidence: &Reservation,
    ) -> Result<(), InvocationDiagnostic> {
        if evidence.effect_id() != effect_id
            || evidence.command_value_ref() != command_ref
            || evidence.domain() != &NonceDomain::from_binding(command.binding())
        {
            return Err(invariant("bind_reservation"));
        }
        Ok(())
    }
}

/// Preparation protocol; authority custody owns command/wire correspondence.
pub struct EvmTransactionPreparationEffect;
impl EffectCapabilityContract for EvmTransactionPreparationEffect {
    type Command = ReservedEvmTransaction;
    type Evidence = PreparedEvmTransactionEvidence;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.evm.capability.prepare-transaction@2")?)
    }
    fn bind_evidence(
        effect_id: &EffectId,
        _: &ContentRef,
        _: &Self::Command,
        _: &ContentRef,
        evidence: &PreparedEvmTransactionEvidence,
    ) -> Result<(), InvocationDiagnostic> {
        if &evidence.effect_id != effect_id {
            return Err(invariant("bind_preparation"));
        }
        Ok(())
    }
}

pub(super) fn invariant(operation: &'static str) -> InvocationDiagnostic {
    InvocationDiagnostic::from_fields(
        "state_internal",
        operation,
        &CapabilityError::EvidenceBinding,
        None,
    )
}

/// Exact semantic request and its corresponding checked native nonce reservation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "reserved-request",
    version = "1",
    schema = "mfm.evm-reserved-request"
)]
pub struct ReservedRequest<R: EvmTransactionRecipe> {
    request: R,
    reserved: ReservedEvmTransaction,
}
impl<R: EvmTransactionRecipe> ReservedRequest<R> {
    /// Checks deterministic request/command correspondence before retaining the pair.
    pub fn new(request: R, reserved: ReservedEvmTransaction) -> Result<Self, InvocationDiagnostic> {
        if &request
            .command()
            .map_err(mfm_capabilities::CallbackFailure::into_diagnostic)?
            != reserved.command()
        {
            return Err(invariant("reserve_request"));
        }
        Ok(Self { request, reserved })
    }
    /// Exact semantic request used to construct the nonce-free command.
    pub fn request(&self) -> &R {
        &self.request
    }
    /// Corresponding checked native command and reservation.
    pub fn reserved(&self) -> &ReservedEvmTransaction {
        &self.reserved
    }
}
impl<'de, R: EvmTransactionRecipe> Deserialize<'de> for ReservedRequest<R> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, bound(deserialize = "R: EvmTransactionRecipe"))]
        struct Wire<R: EvmTransactionRecipe> {
            request: R,
            reserved: ReservedEvmTransaction,
        }
        let wire = Wire::<R>::deserialize(deserializer)?;
        Self::new(wire.request, wire.reserved)
            .map_err(|error| serde::de::Error::custom(format!("{error:?}")))
    }
}

/// Supporting native reservation State, retaining its complete semantic request.
pub struct ReserveEvmNonce<R>(PhantomData<fn() -> R>);
impl<R: EvmTransactionRecipe> State for ReserveEvmNonce<R> {
    type Input = R;
    type Output = ReservedRequest<R>;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.evm.reserve-request@1")?)
    }
}
impl<R: EvmTransactionRecipe> EffectSelection<EvmNonceReservationEffect> for ReserveEvmNonce<R> {
    type ExpandedInput = R;
    type ExpandedOutput = ReservedRequest<R>;
}
impl<R: EvmTransactionRecipe> EffectState<EvmNonceReservationEffect> for ReserveEvmNonce<R> {
    fn prepare(input: &R) -> Result<Eip1559TransactionCommand, InvocationDiagnostic> {
        input
            .command()
            .map_err(mfm_capabilities::CallbackFailure::into_diagnostic)
    }
    fn interpret(
        input: R,
        evidence: &Reservation,
    ) -> Result<ProposedStateOutcome<Self::Output, Never>, InvocationDiagnostic> {
        let command = input
            .command()
            .map_err(mfm_capabilities::CallbackFailure::into_diagnostic)?;
        let reserved = ReservedEvmTransaction::new(command, evidence.clone()).map_err(|error| {
            InvocationDiagnostic::from_fields("state_internal", "reserve_interpret", &error, None)
        })?;
        Ok(ProposedStateOutcome::Success {
            output: ReservedRequest::new(input, reserved)?,
        })
    }
}

/// Supporting native preparation State; signed bytes remain in the transaction authority.
pub struct PrepareEvmTransaction<R>(PhantomData<fn() -> R>);
impl<R: EvmTransactionRecipe> State for PrepareEvmTransaction<R> {
    type Input = ReservedRequest<R>;
    type Output = PreparedTransaction<R>;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.evm.prepare-request@1")?)
    }
}
impl<R: EvmTransactionRecipe> EffectSelection<EvmTransactionPreparationEffect>
    for PrepareEvmTransaction<R>
{
    type ExpandedInput = ReservedRequest<R>;
    type ExpandedOutput = PreparedTransaction<R>;
}
impl<R: EvmTransactionRecipe> EffectState<EvmTransactionPreparationEffect>
    for PrepareEvmTransaction<R>
{
    fn prepare(input: &ReservedRequest<R>) -> Result<ReservedEvmTransaction, InvocationDiagnostic> {
        Ok(input.reserved.clone())
    }
    fn interpret(
        input: ReservedRequest<R>,
        evidence: &PreparedEvmTransactionEvidence,
    ) -> Result<ProposedStateOutcome<Self::Output, Never>, InvocationDiagnostic> {
        let binding_ref = Object::from_value(input.reserved.command().binding())
            .map_err(|error| error.into_diagnostic("prepare_binding"))?
            .value_ref()
            .clone();
        let implementation_ref = mfm_program::effect_implementation_ref::<
            TransactionEffect<R>,
            EvmTransactionImplementation,
        >()
        .map_err(|error| {
            InvocationDiagnostic::from_fields(
                "state_internal",
                "prepare_implementation",
                &error,
                None,
            )
        })?;
        let native = Object::from_value(&PreparedEvmTransaction::new(
            input.reserved,
            evidence.transaction_hash.clone(),
        ))
        .map_err(|error| error.into_diagnostic("prepare_descriptor"))?;
        Ok(ProposedStateOutcome::Success {
            output: PreparedTransaction::new(
                input.request,
                implementation_ref,
                binding_ref,
                native,
            ),
        })
    }
}
