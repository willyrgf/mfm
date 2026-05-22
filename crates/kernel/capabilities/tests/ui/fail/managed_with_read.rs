#[path = "../support/caps.rs"]
mod caps;

use caps::{assert_capability_set, ReadCap};

fn main() {
    assert_capability_set::<mfm_capabilities::ManagedPlatformWrite, (ReadCap,)>();
}
