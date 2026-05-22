use mfm_program_derive::MfmConfig;
use serde::{Deserialize, Serialize};

fn default_name() -> String {
    String::new()
}

#[derive(Clone, Serialize, Deserialize, MfmConfig)]
struct BadConfig {
    #[serde(default = "default_name")]
    name: String,
}

fn main() {}
