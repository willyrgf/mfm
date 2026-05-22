use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
struct BadValue {
    #[serde(skip)]
    hidden: String,
}

fn main() {}
