#[path = "../support/types.rs"]
mod types;

use types::{TryConfig, TryPublicOutputs, TryPureState, TryValue};

fn main() {
    let mut registry = mfm_program::StateRegistryBuilder::new();
    registry.register::<TryPureState>().unwrap();

    let _draft = mfm_program::build_root_with_registry(
        mfm_program::ScopeKey::new("root").unwrap(),
        registry.into_snapshot(),
        |root| {
            let input: mfm_program::Handle<'_, '_, TryValue> =
                root.seed(mfm_program::SeedKey::new("input")?, types::seed()?)?;
            let result = root.scope().state::<TryPureState, _>(
                mfm_program::StateKey::new("state")?,
                TryConfig { multiplier: 2 },
                input,
            )?;
            let outputs = TryPublicOutputs { result };
            root.bind_public_outputs(mfm_program::PublicOutputKey::new("terminal")?, &outputs)
        },
    )
    .unwrap();
}
