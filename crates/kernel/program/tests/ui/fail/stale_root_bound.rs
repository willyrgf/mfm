#[path = "../support/types.rs"]
mod types;

fn main() {
    let mut stale = None;
    let _first = mfm_program::build_root(
        mfm_program::ScopeKey::new("first").unwrap(),
        |root| {
            let handle = root.seed(mfm_program::SeedKey::new("input")?, types::seed()?)?;
            let outputs = types::TryPublicOutputs { result: handle };
            let bound = root.bind_public_outputs(
                mfm_program::PublicOutputKey::new("terminal")?,
                &outputs,
            )?;
            stale = Some(bound);
            Ok(stale.take().unwrap())
        },
    );

    let _second = mfm_program::build_root(
        mfm_program::ScopeKey::new("second").unwrap(),
        |_root| Ok(stale.take().unwrap()),
    );
}
