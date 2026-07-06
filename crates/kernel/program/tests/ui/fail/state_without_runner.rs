#[path = "../support/types.rs"]
mod types;

use types::{TryConfig, TryPublicOutputs, TryValue};

struct StateWithoutRunner;

impl mfm_program::StateSpec for StateWithoutRunner {
    type Config = TryConfig;
    type Context = mfm_program::NoContext;
    type Input = TryValue;
    type Output = TryValue;
    type Effect = mfm_effects::Pure;
    type Caps = mfm_capabilities::NoCaps;

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        mfm_ids::StateKind::new(
            "mfm.program.trybuild.state",
            "state_without_runner",
            mfm_ids::DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.program.trybuild.state:state_without_runner"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        mfm_ids::StateVersion::new("mfm.program.trybuild.state.state_without_runner.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "state_without_runner"
    }

    fn new(_config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

fn main() {
    let mut registry = mfm_program::StateRegistryBuilder::new();
    registry.register::<StateWithoutRunner>().unwrap();

    let _draft = mfm_program::build_root_with_registry(
        mfm_program::ScopeKey::new("root").unwrap(),
        registry.into_snapshot(),
        |root| {
            let input = root.seed(mfm_program::SeedKey::new("input")?, types::seed()?)?;
            let result = root.scope().state::<StateWithoutRunner, _>(
                mfm_program::StateKey::new("state")?,
                mfm_program::NoContext,
                TryConfig { multiplier: 2 },
                input,
            )?;
            let outputs = TryPublicOutputs { result };
            root.bind_public_outputs(mfm_program::PublicOutputKey::new("terminal")?, &outputs)
        },
    )
    .unwrap();
}
