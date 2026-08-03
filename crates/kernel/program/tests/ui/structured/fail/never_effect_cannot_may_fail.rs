#[path = "../support.rs"]
mod support;

use mfm_ids::StableId;
use mfm_program::structured::{
    Direct, Effect, Never, SafeFailureMayFail, State,
};
use support::{EffectCapability, Input, Output};

struct InfallibleEffect;

impl State for InfallibleEffect {
    type Input = Input;
    type Output = Output;
    type Failure = Never;
    type Request = Input;
    type Returned = Output;
    type SafeFailure = Output;
    type Execution = Effect<EffectCapability>;
    type SafeFailureDisposition = SafeFailureMayFail;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(support::stable("mfm.ui/infallible-effect"))
    }
}

fn main() {}
