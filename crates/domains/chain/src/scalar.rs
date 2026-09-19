use mfm_ids::StableId;
use mfm_program::{Classification, ClassifyError, ProposedStateOutcome, PureState, State};
use mfm_program_derive::MfmValue;
use mfm_values::{InvocationDiagnostic, Unsigned256, Unsigned256Error};
use serde::{Deserialize, Serialize};

/// A rejected operand, preserving its originating field and scalar cause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum AdditionInputError {
    /// The left operand failed checked construction.
    #[error("left addition operand is invalid")]
    Left(#[source] Unsigned256Error),
    /// The right operand failed checked construction.
    #[error("right addition operand is invalid")]
    Right(#[source] Unsigned256Error),
}

/// Two complete checked operands for unsigned 256-bit addition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "checked-addition",
    version = "1",
    schema = "mfm.chain-checked-addition"
)]
pub struct CheckedAddition {
    left: Unsigned256,
    right: Unsigned256,
}

impl CheckedAddition {
    /// Parses both complete operands before the input can be admitted.
    pub fn new(
        left: impl Into<String>,
        right: impl Into<String>,
    ) -> Result<Self, AdditionInputError> {
        Ok(Self {
            left: Unsigned256::new(left).map_err(AdditionInputError::Left)?,
            right: Unsigned256::new(right).map_err(AdditionInputError::Right)?,
        })
    }
}

/// Addition exceeded the unsigned 256-bit range; the State call retains both operands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "addition-overflow",
    version = "1",
    schema = "mfm.chain-addition-overflow"
)]
pub struct AdditionOverflow {}

impl ClassifyError for AdditionOverflow {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}

/// Deterministic full-width addition with its own original overflow failure.
pub struct CheckedAdd;
impl State for CheckedAdd {
    type Input = CheckedAddition;
    type Output = Unsigned256;
    type Failure = AdditionOverflow;

    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.chain.checked-add@1")?)
    }
}
impl PureState for CheckedAdd {
    fn evaluate(
        input: CheckedAddition,
    ) -> Result<ProposedStateOutcome<Unsigned256, AdditionOverflow>, InvocationDiagnostic> {
        Ok(match input.left.checked_add(&input.right) {
            Some(output) => ProposedStateOutcome::Success { output },
            None => ProposedStateOutcome::Failure {
                failure: AdditionOverflow {},
            },
        })
    }
}
