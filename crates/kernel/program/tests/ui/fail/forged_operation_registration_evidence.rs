#[path = "../support/types.rs"]
mod types;

use std::marker::PhantomData;

use types::TryOperation;

fn main() {
    let _forged = mfm_program::OperationRegistrationEvidence::<TryOperation> {
        _operation: PhantomData,
        _private: (),
    };
}
