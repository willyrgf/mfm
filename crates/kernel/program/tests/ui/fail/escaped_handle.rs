#![allow(unused_assignments)]

#[path = "../support/types.rs"]
mod types;

fn main() {
    let mut escaped = None;
    let _draft = mfm_program::build_root(
        mfm_program::ScopeKey::new("root").unwrap(),
        |root| {
            let handle = root.seed(mfm_program::SeedKey::new("input")?, types::seed()?)?;
            escaped = Some(handle);
            root.bind_public_outputs(
                mfm_program::PublicOutputKey::new("terminal")?,
                &types::TryPublicOutputs {
                    result: escaped.unwrap(),
                },
            )
        },
    );
}
