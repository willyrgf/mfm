use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BadEnum {
    Tuple(String),
}

fn main() {}
