use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.program.trybuild",
    name = "try_value",
    version = "1",
    schema = "mfm.program.trybuild.try_value"
)]
pub struct TryValue {
    pub amount: u64,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.program.trybuild.public_outputs")]
pub struct TryPublicOutputs<'p, 's> {
    pub result: mfm_program::Handle<'p, 's, TryValue>,
}

#[derive(Clone, Serialize, Deserialize, MfmConfig)]
#[mfm(schema = "mfm.program.trybuild.try_config")]
pub struct TryConfig {
    pub multiplier: u64,
}

pub struct TryPureState {
    pub config: TryConfig,
}

impl mfm_program::StateSpec for TryPureState {
    type Config = TryConfig;
    type Input = TryValue;
    type Output = TryValue;
    type Effect = mfm_effects::Pure;
    type Caps = mfm_capabilities::NoCaps;

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        mfm_ids::StateKind::new(
            "mfm.program.trybuild.state",
            "try_pure_state",
            mfm_ids::DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.program.trybuild.state:try_pure_state"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        mfm_ids::StateVersion::new("mfm.program.trybuild.state.try_pure_state.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "try_pure_state"
    }

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        Ok(Self { config })
    }
}

impl mfm_program::PureState for TryPureState {
    fn run(&self, input: Self::Input) -> mfm_program::StateResult<Self::Output> {
        Ok(TryValue {
            amount: input.amount * self.config.multiplier,
        })
    }
}

pub fn seed() -> mfm_program::Result<mfm_program::CanonicalSeed<TryValue>> {
    mfm_program::CanonicalSeed::from_value(&TryValue { amount: 1 })
}
