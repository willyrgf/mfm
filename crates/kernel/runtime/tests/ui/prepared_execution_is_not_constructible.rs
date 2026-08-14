use std::num::NonZeroU16;

use mfm_capabilities::{AccessCapabilityContract, NoPriorFacts, ReadMode};
use mfm_ids::StableId;
use mfm_program::{FailureValue, ProgramError, State};
use mfm_program_derive::MfmValue as DeriveMfmValue;
use mfm_runtime::PreparedExecution;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, DeriveMfmValue)]
struct TestValue {
    value: u64,
}

impl FailureValue for TestValue {
    fn integrity_blocked() -> Self {
        Self { value: 0 }
    }
}

struct TestState;

impl State for TestState {
    type Input = TestValue;
    type Output = TestValue;
    type Failure = TestValue;

    fn state_id() -> Result<StableId, ProgramError> {
        StableId::new("mfm.test.api-state").map_err(|_| ProgramError::InvalidContract)
    }
}

struct TestCapability;

impl AccessCapabilityContract for TestCapability {
    type Mode = ReadMode;
    type Intent = TestValue;
    type Evidence = TestValue;
    type Facts = NoPriorFacts;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.api-capability")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }

    fn total_attempt_bound() -> NonZeroU16 {
        NonZeroU16::new(1).expect("nonzero")
    }

    fn bind_evidence(
        _intent: &Self::Intent,
        _evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        Ok(())
    }
}

fn main() {
    let _ = PreparedExecution::<TestState, TestCapability> {};
}
