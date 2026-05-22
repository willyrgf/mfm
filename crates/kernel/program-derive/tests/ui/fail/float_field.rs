use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
struct FloatingValue {
    amount: f64,
}

fn main() {}
