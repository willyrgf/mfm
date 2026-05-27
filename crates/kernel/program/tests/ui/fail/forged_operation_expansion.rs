#[path = "../support/types.rs"]
mod types;

use std::marker::PhantomData;
use std::ptr::NonNull;

fn scope<'program, 'scope>(
) -> NonNull<mfm_program::ScopeBuilder<'program, 'scope>> {
    panic!("private field should reject before this is usable")
}

fn main() {
    let _forged = mfm_program::OperationExpansion {
        scope: scope(),
        _program: PhantomData,
        _scope: PhantomData,
        _not_send_sync: PhantomData,
    };
}
