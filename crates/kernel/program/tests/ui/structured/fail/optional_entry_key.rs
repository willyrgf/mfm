// `entry_key` is total, so a request carrying `Option<K>` cannot implement it
// honestly. That is the autoincrement case in its other disguise, and it must
// fail at the type level rather than reach an unreachable branch at runtime.
#[path = "../support.rs"]
mod support;

use mfm_capabilities::EntryKeyed;
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};
use support::InsertKey;

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.ui",
    name = "structured_optional_key_request",
    version = "1",
    schema = "mfm.ui.structured_optional_key_request"
)]
struct OptionalKeyRequest {
    key: Option<InsertKey>,
}

impl EntryKeyed for OptionalKeyRequest {
    type EntryKey = InsertKey;

    fn entry_key(&self) -> Self::EntryKey {
        self.key.clone()
    }
}

fn main() {}
