use super::*;

/// Finite terminal-report projection without duplicating the execution schema per outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-report-outcome",
    version = "1",
    schema = "mfm.evm-transaction-report-outcome"
)]
pub enum TransactionReportOutcome {
    /// Successful creation with its required address.
    Created(Created),
    /// Successful call with its required target.
    Called(Called),
    /// Authenticated reversion already recorded in the settlement.
    Reverted,
}

/// Lossless transaction-local terminal report, with the execution schema stored once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-report-facts",
    version = "1",
    schema = "mfm.evm-transaction-report-facts"
)]
pub struct TransactionReportFacts {
    executed: ExecutedTransactionFacts,
    outcome: TransactionReportOutcome,
}
impl TransactionReportFacts {
    /// Projects checked executed facts into a finite success-or-reversion reporting record.
    pub fn from_executed(
        executed: ExecutedTransactionFacts,
    ) -> Result<Self, mfm_program::StateExecutionError> {
        let outcome = match executed.settlement().outcome() {
            EvmTransactionOutcome::Created { .. } => {
                TransactionReportOutcome::Created(Created::project(&executed)?)
            }
            EvmTransactionOutcome::Called => {
                TransactionReportOutcome::Called(Called::project(&executed)?)
            }
            EvmTransactionOutcome::Reverted => TransactionReportOutcome::Reverted,
        };
        Ok(Self { executed, outcome })
    }
    /// Returns every retained public execution-stage fact.
    pub const fn executed(&self) -> &ExecutedTransactionFacts {
        &self.executed
    }
    /// Returns the checked finite reporting outcome.
    pub const fn outcome(&self) -> &TransactionReportOutcome {
        &self.outcome
    }
}
impl From<CompletedTransactionFacts<Created>> for TransactionReportFacts {
    fn from(facts: CompletedTransactionFacts<Created>) -> Self {
        let (executed, outcome) = facts.into_parts();
        Self {
            executed,
            outcome: TransactionReportOutcome::Created(outcome),
        }
    }
}
impl From<CompletedTransactionFacts<Called>> for TransactionReportFacts {
    fn from(facts: CompletedTransactionFacts<Called>) -> Self {
        let (executed, outcome) = facts.into_parts();
        Self {
            executed,
            outcome: TransactionReportOutcome::Called(outcome),
        }
    }
}
impl<'de> Deserialize<'de> for TransactionReportFacts {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            executed: ExecutedTransactionFacts,
            outcome: TransactionReportOutcome,
        }
        let wire = Wire::deserialize(deserializer)?;
        let report = Self::from_executed(wire.executed).map_err(de::Error::custom)?;
        if report.outcome != wire.outcome {
            return Err(de::Error::custom("transaction report outcome mismatch"));
        }
        Ok(report)
    }
}
