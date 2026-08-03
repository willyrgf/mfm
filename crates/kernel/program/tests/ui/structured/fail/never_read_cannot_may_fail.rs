#[path = "../support.rs"]
mod support;

use mfm_ids::StableId;
use mfm_program::structured::{
    Direct, Never, Read, SafeFailureMayFail, State,
};
use support::{Input, Output, ReadCapability};

struct InfallibleRead;

impl State for InfallibleRead {
    type Input = Input;
    type Output = Output;
    type Failure = Never;
    type Request = Input;
    type Returned = Output;
    type SafeFailure = Output;
    type Execution = Read<ReadCapability>;
    type SafeFailureDisposition = SafeFailureMayFail;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(support::stable("mfm.ui/infallible-read"))
    }
}

fn main() {}
