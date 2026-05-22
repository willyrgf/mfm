use mfm_program_derive::PublicOutputs;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PublicOutputs)]
struct FloatingOutput {
    amount: f64,
}

fn main() {}
