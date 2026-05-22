#[path = "../support/caps.rs"]
mod caps;

use caps::{assert_capability_set, ManagedWriteCap, MutationCap, ReadCap, SupportCap};

fn main() {
    assert_capability_set::<mfm_capabilities::Pure, mfm_capabilities::NoCaps>();
    assert_capability_set::<mfm_capabilities::ReadExternal, (ReadCap,)>();
    assert_capability_set::<mfm_capabilities::ReadExternal, (ReadCap, SupportCap)>();
    assert_capability_set::<mfm_capabilities::ManagedPlatformWrite, (ManagedWriteCap,)>();
    assert_capability_set::<mfm_capabilities::ApplySideEffect, (MutationCap,)>();
    assert_capability_set::<
        mfm_capabilities::ApplySideEffect,
        (ReadCap, MutationCap, SupportCap),
    >();
}
