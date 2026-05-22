#[path = "../support/types.rs"]
mod types;

use mfm_program_derive::PublicOutputs;

#[derive(PublicOutputs)]
struct Swapped<'p, 's> {
    result: mfm_program::Handle<'s, 'p, types::TryValue>,
}

fn main() {}
