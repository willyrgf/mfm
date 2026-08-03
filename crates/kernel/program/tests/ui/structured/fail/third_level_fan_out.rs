#[path = "../support.rs"]
mod support;

use mfm_program::structured::{FanOutResults, Never, OperationBuilder};
use support::{stable, Input};

fn main() -> mfm_program::Result<()> {
    type Inner = FanOutResults<Input, Never>;
    type Outer = FanOutResults<Inner, Never>;

    let mut operation =
        OperationBuilder::<Outer, Never>::new(stable("mfm.ui/depth-three"), stable("root"))?;
    let input = operation.input::<Input>(stable("input"))?;
    let mut outer = operation.root().fan_out::<Inner, Never>(stable("outer"))?;
    outer.lane(stable("outer-lane"), |outer_lane| {
        let mut inner = outer_lane.fan_out::<Input, Never>(stable("inner"))?;
        inner.lane(stable("inner-lane"), |inner_lane| {
            let _third = inner_lane.fan_out::<Input, Never>(stable("too-deep"))?;
            inner_lane.normal(&input)
        })?;
        let joined = inner.finish()?;
        outer_lane.normal(&joined)
    })?;
    Ok(())
}
