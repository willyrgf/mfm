use mfm_program_derive::MfmValue;
use mfm_signing::SignerProviderRuntimeConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "bad_provider_value",
    version = "1",
    schema = "mfm.trybuild.bad_provider_value"
)]
struct BadProviderValue {
    runtime: SignerProviderRuntimeConfig,
}

fn main() {}
