use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
struct Secret<T>(T);

#[derive(Clone, Serialize, Deserialize, MfmValue)]
struct BadValue {
    secret: Secret<String>,
}

fn main() {}
