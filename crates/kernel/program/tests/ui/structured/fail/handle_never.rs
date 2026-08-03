use mfm_ids::StableId;
use mfm_program::structured::{
    Direct, Never, PendingState, Pure, SafeFailureNotApplicable, State,
};
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.ui",
    name = "never_input",
    version = "1",
    schema = "mfm.ui.never_input"
)]
struct Input {
    value: u64,
}

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.ui",
    name = "never_output",
    version = "1",
    schema = "mfm.ui.never_output"
)]
struct Output {
    value: u64,
}

struct Infallible;

impl State for Infallible {
    type Input = Input;
    type Output = Output;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.ui/infallible")
            .map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
    }
}

fn attach_handler(pending: PendingState<'_, Infallible, Never>) {
    let _ = pending.or_default();
}

fn main() {}
