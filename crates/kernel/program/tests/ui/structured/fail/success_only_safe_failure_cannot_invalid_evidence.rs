#[path = "../support.rs"]
mod support;

use std::sync::Arc;

use mfm_ids::StableId;
use mfm_program::structured::{
    Direct, Read, SafeFailureSuccessOnly, State, StateFrame, StateSettlement,
    StructuredStateCallbacks,
};
use mfm_spec::structured::ProposedStateOutcome;
use support::{Input, Output, ReadCapability};

struct SuccessOnlyRead;

impl State for SuccessOnlyRead {
    type Input = Input;
    type Output = Output;
    type Failure = Output;
    type Request = Input;
    type Returned = Output;
    type SafeFailure = Output;
    type Execution = Read<ReadCapability>;
    type SafeFailureDisposition = SafeFailureSuccessOnly;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(support::stable("mfm.ui/success-only-read-invalid"))
    }
}

fn main() {
    let _callbacks = StructuredStateCallbacks::<SuccessOnlyRead>::Read {
        request: Arc::new(|frame: StateFrame<'_, Input>| frame.input().clone()),
        settle_returned: Arc::new(|_frame, returned| {
            StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
        }),
        // InvalidEvidence is not a SafeFailureSuccessOnly proposal.
        settle_safe_failure: Arc::new(|_frame, _failure| StateSettlement::InvalidEvidence),
    };
}
