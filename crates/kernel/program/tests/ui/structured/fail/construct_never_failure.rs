#[path = "../support.rs"]
mod support;

use mfm_program::structured::{
    BlockBuilder, FanOutResults, Never, OperationBuilder, Value,
};
use support::{stable, Output};

type KernelNever = Never;

fn root_failure(
    operation: &mut OperationBuilder<Output, KernelNever>,
    failure: &Value<KernelNever>,
) {
    let _ = operation.fail(failure);
}

fn scope_failure(
    block: &BlockBuilder<KernelNever>,
    failure: &Value<KernelNever>,
) {
    let _ = block.scope_failure::<Output>(failure);
}

fn lane_failure(failure: &Value<KernelNever>) -> mfm_program::Result<()> {
    type Join = FanOutResults<Output, KernelNever>;

    let mut operation =
        OperationBuilder::<Join, KernelNever>::new(stable("mfm.ui/never-lane"), stable("root"))?;
    let mut group = operation
        .root()
        .fan_out::<Output, KernelNever>(stable("group"))?;
    group.lane(stable("lane"), |lane| {
        lane.scope_failure::<Output>(failure)
    })?;
    Ok(())
}

fn main() {}
