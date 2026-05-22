use mfm_program_derive::MfmConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Serialize, Deserialize, MfmConfig)]
struct BadConfig {
    weights: HashMap<String, u64>,
}

fn main() {}
