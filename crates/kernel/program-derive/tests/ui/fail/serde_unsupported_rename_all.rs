use mfm_program_derive::MfmConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmConfig)]
#[serde(rename_all = "PascalCase")]
struct BadConfig {
    account_id: String,
}

fn main() {}
