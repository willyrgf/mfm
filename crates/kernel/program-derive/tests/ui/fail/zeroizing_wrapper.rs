use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
struct Zeroizing<T>(T);

#[derive(Clone, Serialize, Deserialize, MfmValue)]
struct BadValue {
    secret: Zeroizing<String>,
}

fn main() {}
