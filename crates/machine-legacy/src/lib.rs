pub mod state;
pub mod state_machine;
// Allow proc-macro expansions to refer to this crate as `::mfm_machine`, even when
// they are used within the crate itself.
extern crate mfm_machine_derive;
extern crate self as mfm_machine;

//FIXME: reorganize library to be more ergonomic to use
// see example in tests/public_api_test.rs how bad its.
