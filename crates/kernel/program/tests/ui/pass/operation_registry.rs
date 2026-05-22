#[path = "../support/types.rs"]
mod types;

use mfm_program::{build_root_with_registries, OperationKey, OperationRegistryBuilder};
use types::{TryConfig, TryOperation, TryPublicOutputs, TryPureState};

fn main() -> mfm_program::Result<()> {
    let mut states = mfm_program::StateRegistryBuilder::new();
    states.register::<TryPureState>().unwrap();
    let mut operations = OperationRegistryBuilder::new();
    operations.register::<TryOperation>().unwrap();

    let _draft = build_root_with_registries(
        mfm_program::ScopeKey::new("root")?,
        states.into_snapshot(),
        operations.into_snapshot(),
        |root| {
            let input = root.seed(mfm_program::SeedKey::new("seed")?, types::seed()?)?;
            let result = root.scope().call::<TryOperation, _>(
                OperationKey::new("try-operation")?,
                TryOperation,
                TryConfig { multiplier: 4 },
                input,
            )?;
            let outputs = TryPublicOutputs {
                result: result.result,
            };
            root.bind_public_outputs(mfm_program::PublicOutputKey::new("terminal")?, &outputs)
        },
    )?;

    Ok(())
}
