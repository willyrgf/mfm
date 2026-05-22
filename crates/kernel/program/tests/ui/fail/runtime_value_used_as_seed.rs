#[path = "../support/types.rs"]
mod types;

fn main() {
    let _draft = mfm_program::build_root(
        mfm_program::ScopeKey::new("root").unwrap(),
        |root| {
            let handle = root.seed(mfm_program::SeedKey::new("input")?, types::seed()?)?;
            let _bad = root.seed(mfm_program::SeedKey::new("runtime")?, handle)?;
            unreachable!()
        },
    );
}
