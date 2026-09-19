use std::marker::PhantomData;

use mfm_capabilities::ReadCapabilityContract;
use mfm_ids::StableId;
use mfm_program::{
    Classification, ClassifyError, ProposedStateOutcome, ReadSelection, ReadState, State,
};
use mfm_program_derive::MfmValue;
use mfm_values::{InvocationDiagnostic, MfmValue as Value, Object};
use serde::{Deserialize, Serialize};

use super::{
    BalanceContext, BalanceEvidence, BalanceOutcome, BalanceRead, CandidateBalance,
    PreparedBalance, ReadBalanceAt,
};

/// Authenticated unsuccessful balance observation, distinct from local admission errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
pub enum ObserveBalanceFailure {
    /// The native protocol authenticated rejection or absence.
    ObservationUnavailable,
    /// Authenticated evidence violated integrity constraints.
    IntegrityBlocked,
}
impl ObserveBalanceFailure {
    /// Projects the shared public failure code while retaining this exact original separately.
    pub const fn code(&self) -> super::BalanceFailureCode {
        match self {
            Self::ObservationUnavailable => super::BalanceFailureCode::ObservationUnavailable,
            Self::IntegrityBlocked => super::BalanceFailureCode::IntegrityBlocked,
        }
    }
}
impl ClassifyError for ObserveBalanceFailure {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}

/// Observes the prepared source while preserving its exactly typed caller continuation.
pub struct ObserveBalance<K>(PhantomData<fn() -> K>);
impl<K: Value> State for ObserveBalance<K> {
    type Input = PreparedBalance<K>;
    type Output = CandidateBalance<K>;
    type Failure = ObserveBalanceFailure;
    fn description() -> &'static str {
        "Observes one selected balance through its native implementation."
    }
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.chain.observe-balance@1")?)
    }
}
impl<K: Value> ReadSelection<BalanceRead> for ObserveBalance<K> {
    type ExpandedInput = BalanceContext<K>;
    type ExpandedOutput = BalanceContext<K>;
}
impl<K: Value> ReadState<BalanceRead> for ObserveBalance<K> {
    fn prepare(input: &PreparedBalance<K>) -> Result<ReadBalanceAt, InvocationDiagnostic> {
        let source = input.context().active_source().map_err(|source| {
            InvocationDiagnostic::from_fields("state_internal", "prepare_balance", &source, None)
        })?;
        ReadBalanceAt::new(
            source.target().clone(),
            input.observed_at().clone(),
            input.context().metadata().route_ref().clone(),
        )
        .map_err(|source| {
            InvocationDiagnostic::from_fields("state_internal", "prepare_balance", &source, None)
        })
    }
    fn interpret(
        input: PreparedBalance<K>,
        evidence: &BalanceEvidence,
    ) -> Result<
        ProposedStateOutcome<CandidateBalance<K>, ObserveBalanceFailure>,
        InvocationDiagnostic,
    > {
        let intent = Self::prepare(&input)?;
        let original = Object::from_value(&intent)
            .map_err(|source| source.into_diagnostic("balance_intent_identity"))?;
        BalanceRead::bind_evidence(
            original.value_ref(),
            &intent,
            evidence.original().value_ref(),
            evidence,
        )?;
        Ok(match evidence.outcome() {
            BalanceOutcome::Observed { raw_units, .. } => ProposedStateOutcome::Success {
                output: CandidateBalance::new(input, raw_units.clone()),
            },
            BalanceOutcome::Rejected | BalanceOutcome::SafeFailure => {
                ProposedStateOutcome::Failure {
                    failure: ObserveBalanceFailure::ObservationUnavailable,
                }
            }
            BalanceOutcome::IntegrityBlocked => ProposedStateOutcome::Failure {
                failure: ObserveBalanceFailure::IntegrityBlocked,
            },
        })
    }
}
