use mfm_store::ResolvedConfiguration;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, mfm_program_derive::MfmConfig)]
#[serde(deny_unknown_fields)]
struct ConfigA {}

#[derive(Serialize, Deserialize, mfm_program_derive::MfmConfig)]
#[serde(deny_unknown_fields)]
struct ConfigB {}

fn needs_b(_value: ResolvedConfiguration<ConfigB>) {}

fn cross(value: ResolvedConfiguration<ConfigA>) {
    needs_b(value);
}

fn main() {}
