use mfm_program_derive::MfmConfig;
use mfm_signing::SignerProviderRuntimeConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmConfig)]
struct BadProviderConfig {
    runtime: SignerProviderRuntimeConfig,
}

fn main() {}
