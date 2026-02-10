pub mod state;
pub mod state_machine;
// Allow proc-macro expansions to refer to this crate by name (even within the crate itself).
extern crate mfm_machine_derive_legacy;
extern crate self as mfm_machine_legacy;

//FIXME: reorganize library to be more ergonomic to use
// see example in tests/public_api_test.rs how bad its.
