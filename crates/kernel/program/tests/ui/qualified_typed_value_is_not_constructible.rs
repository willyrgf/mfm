use mfm_program::QualifiedTypedValue;
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, MfmValue)]
struct TestValue {
    value: u64,
}

fn main() {
    let _ = QualifiedTypedValue::<TestValue> {};
}
