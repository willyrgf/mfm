use mfm_program_derive::{MfmValue, PublicOutputs};
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

pub fn seed() -> mfm_program::Result<mfm_program::CanonicalSeed<TryValue>> {
    mfm_program::CanonicalSeed::from_value(&TryValue { amount: 1 })
}
