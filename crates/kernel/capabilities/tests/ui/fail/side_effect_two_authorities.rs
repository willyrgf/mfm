#[path = "../support/caps.rs"]
mod caps;

use caps::{assert_capability_set, MutationCap};

fn main() {
    assert_capability_set::<mfm_capabilities::ApplySideEffect, (MutationCap, MutationCap)>();
}
