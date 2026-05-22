#[path = "../support/types.rs"]
mod types;

use std::marker::PhantomData;

use types::TryPureState;

fn descriptor() -> mfm_program::StateDescriptorIdentity {
    panic!("private field should reject before this is usable")
}

fn evidence() -> mfm_program::StateRegistrationEvidence<TryPureState> {
    panic!("private field should reject before this is usable")
}

fn main() {
    let _forged = mfm_program::RegisteredState::<TryPureState> {
        descriptor: descriptor(),
        runner: mfm_program::RunnerKind::Pure,
        evidence: evidence(),
        _state: PhantomData,
    };
}
