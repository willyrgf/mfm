//! Nominal fixture stages for source connectivity only; these are not lifecycle contracts.

use mfm_capabilities::{EffectCapabilityContract, ReadCapabilityContract};
use mfm_ids::StableId;
use mfm_program::{EffectState, Never, ProposedStateOutcome, PureState, ReadState, State};
use mfm_program_derive::MfmValue;
use mfm_values::InvocationDiagnostic;
use serde::{Deserialize, Serialize};

use mfm_program::{EffectSelection, ReadSelection};

macro_rules! value {
    ($($name:ident),+) => {$(
        #[derive(Debug, Serialize, Deserialize, MfmValue)]
        #[serde(deny_unknown_fields)]
        pub struct $name { pub value: u64 }
    )+};
}
value!(Input, Prepared, Deployed, Configured, Observed, Validated, Report);

pub struct Transaction;
impl EffectCapabilityContract for Transaction {
    type Command = Prepared;
    type Evidence = Deployed;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("proof.transaction@1")?)
    }
    fn bind_evidence(
        _: &mfm_ids::EffectId,
        _: &mfm_ids::ContentRef,
        _: &Prepared,
        _: &mfm_ids::ContentRef,
        _: &Deployed,
    ) -> Result<(), InvocationDiagnostic> {
        Ok(())
    }
}

pub struct Observation;
impl ReadCapabilityContract for Observation {
    type Intent = Configured;
    type Evidence = Observed;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("proof.observation@1")?)
    }
    fn bind_evidence(
        _: &mfm_ids::ContentRef,
        _: &Configured,
        _: &mfm_ids::ContentRef,
        _: &Observed,
    ) -> Result<(), InvocationDiagnostic> {
        Ok(())
    }
}

macro_rules! state {
    ($name:ident, $input:ty, $output:ty) => {
        pub struct $name;
        impl State for $name {
            type Input = $input;
            type Output = $output;
            type Failure = Never;
            fn state_id() -> mfm_program::Result<StableId> {
                Ok(StableId::new(
                    concat!("proof.", stringify!($name), "@1").to_ascii_lowercase(),
                )?)
            }
        }
    };
}
state!(Deploy, Prepared, Deployed);
state!(Configure, Deployed, Configured);
state!(Observe, Configured, Observed);
state!(Validate, Observed, Validated);
state!(Finish, Validated, Report);
state!(Add, Deployed, Deployed);
state!(Custom, Deployed, Deployed);

impl EffectState<Transaction> for Deploy {
    fn prepare(input: &Prepared) -> Result<Prepared, InvocationDiagnostic> {
        Ok(Prepared { value: input.value })
    }
    fn interpret(
        _: Prepared,
        evidence: &Deployed,
    ) -> Result<ProposedStateOutcome<Deployed, Never>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Success {
            output: Deployed {
                value: evidence.value,
            },
        })
    }
}
impl EffectSelection<Transaction> for Deploy {
    type ExpandedInput = Input;
    type ExpandedOutput = Deployed;
}
impl EffectState<Transaction> for Configure {
    fn prepare(input: &Deployed) -> Result<Prepared, InvocationDiagnostic> {
        Ok(Prepared { value: input.value })
    }
    fn interpret(
        _: Deployed,
        evidence: &Deployed,
    ) -> Result<ProposedStateOutcome<Configured, Never>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Success {
            output: Configured {
                value: evidence.value,
            },
        })
    }
}
impl EffectSelection<Transaction> for Configure {
    type ExpandedInput = Deployed;
    type ExpandedOutput = Configured;
}
impl ReadState<Observation> for Observe {
    fn prepare(input: &Configured) -> Result<Configured, InvocationDiagnostic> {
        Ok(Configured { value: input.value })
    }
    fn interpret(
        _: Configured,
        evidence: &Observed,
    ) -> Result<ProposedStateOutcome<Observed, Never>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Success {
            output: Observed {
                value: evidence.value,
            },
        })
    }
}
impl ReadSelection<Observation> for Observe {
    type ExpandedInput = Configured;
    type ExpandedOutput = Observed;
}
macro_rules! pure {
    ($name:ident, $input:ident, $output:ident) => {
        impl PureState for $name {
            fn evaluate(
                input: $input,
            ) -> Result<ProposedStateOutcome<$output, Never>, InvocationDiagnostic> {
                Ok(ProposedStateOutcome::Success {
                    output: $output { value: input.value },
                })
            }
        }
    };
}
pure!(Add, Deployed, Deployed);
pure!(Custom, Deployed, Deployed);
pure!(Validate, Observed, Validated);
pure!(Finish, Validated, Report);
