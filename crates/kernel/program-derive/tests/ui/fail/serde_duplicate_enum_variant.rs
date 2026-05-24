use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum BadEnum {
    #[serde(rename = "dup")]
    First,
    #[serde(rename = "dup")]
    Second,
}

fn main() {}
