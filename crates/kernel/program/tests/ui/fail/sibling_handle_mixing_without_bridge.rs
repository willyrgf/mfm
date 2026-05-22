#[path = "../support/types.rs"]
mod types;

fn require_same_scope<'p, 's>(
    _left: mfm_program::Handle<'p, 's, types::TryValue>,
    _right: mfm_program::Handle<'p, 's, types::TryValue>,
) {
}

fn main() {
    let mut first_child_handle = None;
    let _draft = mfm_program::build_root(
        mfm_program::ScopeKey::new("root").unwrap(),
        |root| {
            let parent =
                root.seed(mfm_program::SeedKey::new("input")?, types::seed()?)?;
            let _first = root.scope().child_scope(
                mfm_program::ScopeKey::new("first")?,
                |child| {
                    let imported = child.import_from_parent(
                        mfm_program::BridgeKey::new("import")?,
                        parent.clone(),
                        mfm_program::BridgePolicy::same_run_same_value(),
                    )?;
                    first_child_handle = Some(imported);
                    child.bridge_to_parent(parent.clone())
                },
            )?;
            let second = root.scope().child_scope(
                mfm_program::ScopeKey::new("second")?,
                |child| {
                    let second_child = child.import_from_parent(
                        mfm_program::BridgeKey::new("import")?,
                        parent.clone(),
                        mfm_program::BridgePolicy::same_run_same_value(),
                    )?;
                    require_same_scope(first_child_handle.take().unwrap(), second_child);
                    child.bridge_to_parent(parent.clone())
                },
            )?;
            let outputs = types::TryPublicOutputs { result: second };
            root.bind_public_outputs(mfm_program::PublicOutputKey::new("terminal")?, &outputs)
        },
    );
}
