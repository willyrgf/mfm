#[path = "../support/types.rs"]
mod types;

use mfm_program::IntoStateInput;
use mfm_program_derive::StateInput;
use serde::{Deserialize, Serialize};
use types::{TryPublicOutputs, TryValue};

#[derive(Clone, Serialize, Deserialize, StateInput)]
#[serde(rename_all = "camelCase")]
struct TryInput {
    primary_value: TryValue,
    ordered_values: Vec<TryValue>,
}

fn main() {
    let _draft = mfm_program::build_root(
        mfm_program::ScopeKey::new("root").unwrap(),
        |root| {
            let first: mfm_program::Handle<'_, '_, TryValue> =
                root.seed(mfm_program::SeedKey::new("first")?, types::seed()?)?;
            let second: mfm_program::Handle<'_, '_, TryValue> =
                root.seed(mfm_program::SeedKey::new("second")?, types::seed()?)?;
            let input = TryInputHandles {
                primary_value: first.clone(),
                ordered_values: vec![first.clone(), second],
            };
            let _binding: mfm_program::InputBinding<TryInput> = input.into_binding()?;
            let outputs = TryPublicOutputs { result: first };
            root.bind_public_outputs(mfm_program::PublicOutputKey::new("terminal")?, &outputs)
        },
    )
    .unwrap();
}
