use mfm_program_derive::MfmConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmConfig)]
struct BadConfig {
    #[serde(default)]
    name: String,
}

fn main() {}
