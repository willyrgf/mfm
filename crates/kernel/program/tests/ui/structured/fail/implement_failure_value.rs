use mfm_program::structured::FailureValue;
use mfm_spec::structured::StructuredFailureContract;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
enum LookalikeNever {}

impl FailureValue for LookalikeNever {
    fn failure_contract() -> mfm_program::Result<StructuredFailureContract> {
        Ok(StructuredFailureContract::never())
    }
}

fn main() {}
