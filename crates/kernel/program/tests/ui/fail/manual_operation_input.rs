#[path = "../support/types.rs"]
mod types;

struct ForgedInput;

impl<'p, 's> mfm_program::OperationInput<'p, 's> for ForgedInput {
    type Runtime = types::TryValue;

    fn input_binding(&self) -> mfm_program::Result<mfm_program::OperationInputBindingSpec> {
        unimplemented!()
    }
}

fn main() {}
