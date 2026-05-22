#[path = "../support/types.rs"]
mod types;

fn main() {
    let _draft = mfm_program::build_root(
        mfm_program::ScopeKey::new("root").unwrap(),
        |root| {
            let parent =
                root.seed(mfm_program::SeedKey::new("input")?, types::seed()?)?;
            let child_output = root.scope().child_scope(
                mfm_program::ScopeKey::new("child")?,
                |child| {
                    let imported = child.import_from_parent(
                        mfm_program::BridgeKey::new("import")?,
                        parent.clone(),
                        mfm_program::BridgePolicy::same_run_same_value(),
                    )?;
                    child.bridge_to_parent(imported)
                },
            )?;
            let outputs = types::TryPublicOutputs {
                result: child_output,
            };
            root.bind_public_outputs(mfm_program::PublicOutputKey::new("terminal")?, &outputs)
        },
    );
}
