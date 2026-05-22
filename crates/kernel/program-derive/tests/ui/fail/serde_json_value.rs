use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
struct BadValue {
    raw: serde_json::Value,
}

fn main() {}
