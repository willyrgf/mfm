#[path = "../support.rs"]
mod support;

use mfm_program::structured::{FanOutResults, Never, OperationBuilder};
use support::{stable, EffectState, Input, Output};

fn main() -> mfm_program::Result<()> {
    type Join = FanOutResults<Output, Never>;

    let mut operation =
        OperationBuilder::<Join, Never>::new(stable("mfm.ui/effect-lane"), stable("root"))?;
    let input = operation.input::<Input>(stable("input"))?;
    let mut group = operation.root().fan_out::<Output, Never>(stable("group"))?;
    group.lane(stable("lane"), |lane| {
        let output = lane
            .state::<EffectState>(stable("effect"), &input)?
            .infallible()?;
        lane.normal(&output)
    })?;
    Ok(())
}
