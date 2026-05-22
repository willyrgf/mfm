use mfm_program_derive::MfmConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmConfig)]
struct FloatingConfig {
    amount: f64,
}

fn main() {}
