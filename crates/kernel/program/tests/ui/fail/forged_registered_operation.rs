#[path = "../support/types.rs"]
mod types;

use std::marker::PhantomData;

use types::TryOperation;

fn descriptor() -> mfm_program::OperationDescriptorIdentity {
    panic!("private field should reject before this is usable")
}

fn evidence() -> mfm_program::OperationRegistrationEvidence<TryOperation> {
    panic!("private field should reject before this is usable")
}

fn main() {
    let _forged = mfm_program::RegisteredOperation::<TryOperation> {
        descriptor: descriptor(),
        evidence: evidence(),
        _operation: PhantomData,
    };
}
