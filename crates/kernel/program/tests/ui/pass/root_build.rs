#[path = "../support/types.rs"]
mod types;

use types::{TryPublicOutputs, TryValue};

fn main() {
    let _draft = mfm_program::build_root(
        mfm_program::ScopeKey::new("root").unwrap(),
        |root| {
            let handle: mfm_program::Handle<'_, '_, TryValue> =
                root.seed(mfm_program::SeedKey::new("input")?, types::seed()?)?;
            let outputs = TryPublicOutputs { result: handle };
            root.bind_public_outputs(mfm_program::PublicOutputKey::new("terminal")?, &outputs)
        },
    )
    .unwrap();
}
