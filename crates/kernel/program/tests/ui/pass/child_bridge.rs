#[path = "../support/types.rs"]
mod types;

use types::{TryPublicOutputs, TryValue};

fn main() {
    let _draft = mfm_program::build_root(
        mfm_program::ScopeKey::new("root").unwrap(),
        |root| {
            let parent: mfm_program::Handle<'_, '_, TryValue> =
                root.seed(mfm_program::SeedKey::new("input")?, types::seed()?)?;
            let child_output = root.scope().child_scope(
                mfm_program::ScopeKey::new("child")?,
                |child| {
                    let imported = child.import_from_parent(
                        mfm_program::BridgeKey::new("import")?,
                        parent.clone(),
                        mfm_program::BridgePolicy::same_run_same_value(),
                    )?;
                    let exported = child.export_to_parent(
                        mfm_program::BridgeKey::new("export")?,
                        imported,
                        mfm_program::BridgePolicy::same_run_same_value(),
                    )?;
                    child.bridge_to_parent(exported)
                },
            )?;
            let outputs = TryPublicOutputs {
                result: child_output,
            };
            root.bind_public_outputs(mfm_program::PublicOutputKey::new("terminal")?, &outputs)
        },
    )
    .unwrap();
}
