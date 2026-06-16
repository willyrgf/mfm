#[path = "../support/types.rs"]
mod types;

use mfm_program::{Operation, ScopeBuilder};
use types::{TryConfig, TryOperation, TryOperationOutputs, TryValue};

fn call_expand_directly<'program, 'scope>(
    operation: &TryOperation,
    input: mfm_program::Handle<'program, 'scope, TryValue>,
    builder: &mut ScopeBuilder<'program, 'scope>,
) -> mfm_program::Result<TryOperationOutputs<'program, 'scope>> {
    let config = mfm_program::ValidatedConfig::new(TryConfig { multiplier: 2 }).unwrap();
    operation.expand(config, input, builder)
}

fn main() {}
