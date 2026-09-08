use super::*;
use mfm_program::StateExecutionError;

/// Reservation and the complete accepted preparation evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "prepared-transaction-facts",
    version = "1",
    schema = "mfm.evm-prepared-transaction-facts"
)]
pub struct PreparedTransactionFacts {
    reserved: ReservedEvmTransaction,
    preparation: PreparedEvmTransactionEvidence,
}
impl PreparedTransactionFacts {
    /// Retains checked reservation facts and the accepted public preparation evidence.
    ///
    /// The preparation EffectId's relationship to a run belongs to Runtime qualification.
    pub const fn new(
        reserved: ReservedEvmTransaction,
        preparation: PreparedEvmTransactionEvidence,
    ) -> Self {
        Self {
            reserved,
            preparation,
        }
    }
    /// Returns all reservation-stage facts.
    pub const fn reserved(&self) -> &ReservedEvmTransaction {
        &self.reserved
    }
    /// Returns the complete nonce-free command.
    pub const fn command(&self) -> &Eip1559TransactionCommand {
        self.reserved.command()
    }
    /// Returns the complete reservation.
    pub const fn reservation(&self) -> &Reservation {
        self.reserved.reservation()
    }
    /// Returns the complete accepted preparation evidence.
    pub const fn preparation(&self) -> &PreparedEvmTransactionEvidence {
        &self.preparation
    }
    /// Constructs the existing execution capability descriptor without workflow data.
    pub fn execution_command(&self) -> PreparedEvmTransaction {
        PreparedEvmTransaction::new(
            self.reserved.clone(),
            self.preparation.transaction_hash.clone(),
        )
    }
}

/// All public transaction facts through authenticated settlement, including reversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "executed-transaction-facts",
    version = "1",
    schema = "mfm.evm-executed-transaction-facts"
)]
pub struct ExecutedTransactionFacts {
    prepared: PreparedTransactionFacts,
    settlement: EvmTransactionSettlement,
}
impl ExecutedTransactionFacts {
    /// Checks settlement nonce, hash, and action against the complete preparation facts.
    pub fn new(
        prepared: PreparedTransactionFacts,
        settlement: EvmTransactionSettlement,
    ) -> Result<Self, EvmDomainError> {
        let facts = Self {
            prepared,
            settlement,
        };
        facts.validate()?;
        Ok(facts)
    }
    fn validate(&self) -> Result<(), EvmDomainError> {
        if self.reservation().nonce() != self.settlement.nonce()
            || (&self.preparation().transaction_hash) != self.settlement.transaction_hash()
            || !super::stages::action_matches(self.command(), &self.settlement)
        {
            return Err(EvmDomainError::EvidenceBinding);
        }
        Ok(())
    }
    /// Returns all preparation-stage facts.
    pub const fn prepared(&self) -> &PreparedTransactionFacts {
        &self.prepared
    }
    /// Returns the exact retained command.
    pub const fn command(&self) -> &Eip1559TransactionCommand {
        self.prepared.command()
    }
    /// Returns the complete retained reservation.
    pub const fn reservation(&self) -> &Reservation {
        self.prepared.reservation()
    }
    /// Returns the complete retained preparation evidence.
    pub const fn preparation(&self) -> &PreparedEvmTransactionEvidence {
        self.prepared.preparation()
    }
    /// Returns the complete accepted settlement.
    pub const fn settlement(&self) -> &EvmTransactionSettlement {
        &self.settlement
    }
}
impl_checked_deserialize!(ExecutedTransactionFacts {
    prepared: PreparedTransactionFacts,
    settlement: EvmTransactionSettlement
});

mod sealed {
    pub trait Sealed {}
}

/// EVM-owned success projection, sealed to creation and ordinary call outcomes.
pub trait TransactionSuccessMode: sealed::Sealed + MfmValueTrait + Clone + Eq {
    /// Stable outcome discriminator used in executable identity.
    fn mode_id() -> &'static str;
    /// Whether the complete command has the selected action.
    fn accepts(command: &Eip1559TransactionCommand) -> bool;
    /// Projects matching successful evidence; a mismatch is an internal implementation error.
    fn project(executed: &ExecutedTransactionFacts) -> Result<Self, StateExecutionError>;
}

/// Checked required address from a successful creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "created",
    version = "1",
    schema = "mfm.evm-created"
)]
pub struct Created {
    created_address: EvmAddress,
}
impl Created {
    /// Returns the required created address.
    pub const fn created_address(&self) -> &EvmAddress {
        &self.created_address
    }
}
impl sealed::Sealed for Created {}
impl TransactionSuccessMode for Created {
    fn mode_id() -> &'static str {
        "created"
    }
    fn accepts(command: &Eip1559TransactionCommand) -> bool {
        command.to().is_none()
    }
    fn project(executed: &ExecutedTransactionFacts) -> Result<Self, StateExecutionError> {
        match (executed.command().to(), executed.settlement().outcome()) {
            (None, EvmTransactionOutcome::Created { created_address }) => Ok(Self {
                created_address: created_address.clone(),
            }),
            _ => Err(StateExecutionError),
        }
    }
}

/// Checked required target from a successful ordinary call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "called",
    version = "1",
    schema = "mfm.evm-called"
)]
pub struct Called {
    target: EvmAddress,
}
impl Called {
    /// Returns the required call target.
    pub const fn target(&self) -> &EvmAddress {
        &self.target
    }
}
impl sealed::Sealed for Called {}
impl TransactionSuccessMode for Called {
    fn mode_id() -> &'static str {
        "called"
    }
    fn accepts(command: &Eip1559TransactionCommand) -> bool {
        command.to().is_some()
    }
    fn project(executed: &ExecutedTransactionFacts) -> Result<Self, StateExecutionError> {
        match (executed.command().to(), executed.settlement().outcome()) {
            (Some(target), EvmTransactionOutcome::Called) => Ok(Self {
                target: target.clone(),
            }),
            _ => Err(StateExecutionError),
        }
    }
}

/// Complete transaction-local execution facts and their checked successful projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "completed-transaction-facts",
    version = "1",
    schema = "mfm.evm-completed-transaction-facts"
)]
pub struct CompletedTransactionFacts<S: TransactionSuccessMode> {
    executed: ExecutedTransactionFacts,
    outcome: S,
}
impl<S: TransactionSuccessMode> CompletedTransactionFacts<S> {
    /// Projects only a matching successful settlement into a completed fact type.
    pub fn new(executed: ExecutedTransactionFacts) -> Result<Self, StateExecutionError> {
        let outcome = S::project(&executed)?;
        Ok(Self { executed, outcome })
    }
    /// Returns the retained execution facts without the successful-mode projection.
    pub fn into_executed(self) -> ExecutedTransactionFacts {
        self.executed
    }
    /// Returns all public facts through settlement.
    pub const fn executed(&self) -> &ExecutedTransactionFacts {
        &self.executed
    }
    /// Returns the complete original command.
    pub const fn command(&self) -> &Eip1559TransactionCommand {
        self.executed.command()
    }
    /// Returns the complete reservation.
    pub const fn reservation(&self) -> &Reservation {
        self.executed.reservation()
    }
    /// Returns the complete preparation evidence.
    pub const fn preparation(&self) -> &PreparedEvmTransactionEvidence {
        self.executed.preparation()
    }
    /// Returns the complete settlement.
    pub const fn settlement(&self) -> &EvmTransactionSettlement {
        self.executed.settlement()
    }
    /// Returns the checked outcome-specific result.
    pub const fn outcome(&self) -> &S {
        &self.outcome
    }
}
impl<'de, S: TransactionSuccessMode> Deserialize<'de> for CompletedTransactionFacts<S> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(
            deny_unknown_fields,
            bound(deserialize = "S: serde::de::DeserializeOwned")
        )]
        struct Wire<S> {
            executed: ExecutedTransactionFacts,
            outcome: S,
        }
        let wire = Wire::<S>::deserialize(deserializer)?;
        let facts = Self::new(wire.executed).map_err(de::Error::custom)?;
        if facts.outcome != wire.outcome {
            return Err(de::Error::custom("transaction outcome mismatch"));
        }
        Ok(facts)
    }
}

/// Accumulated executed context at the transaction's authenticated reversion branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "C: serde::de::DeserializeOwned")
)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-failure",
    version = "1",
    schema = "mfm.evm-transaction-failure"
)]
pub struct EvmTransactionFailure<C: MfmValueTrait> {
    context: C,
}
impl<C: MfmValueTrait> EvmTransactionFailure<C> {
    pub(super) fn new(context: C) -> Self {
        Self { context }
    }
    /// Borrows the accumulated context whose selected settlement records reversion.
    pub const fn context(&self) -> &C {
        &self.context
    }
    /// Moves the accumulated context into the product's terminal failure policy.
    pub fn into_context(self) -> C {
        self.context
    }
}
