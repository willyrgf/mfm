#[path = "../support.rs"]
mod support;

use mfm_program::structured::{Never, PolicyRecipeBuilder};
use support::{stable, Input, Output};

fn main() -> mfm_program::Result<()> {
    let (mut recipe, input, proceed) = PolicyRecipeBuilder::<Input, Output, Never>::new(
        stable("mfm.ui/fan-out-proceed"),
        stable("root"),
    )?;
    let mut group = recipe.root().fan_out::<Output, Never>(stable("group"))?;
    group.lane(stable("lane"), move |lane| {
        let output = proceed.call(lane, stable("proceed"), &input)?;
        lane.normal(&output)
    })?;
    Ok(())
}
