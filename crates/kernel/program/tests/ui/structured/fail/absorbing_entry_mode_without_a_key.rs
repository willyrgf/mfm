// Absorption is unavailable without an entry key: an autoincrement insert has
// no value to name, so declaring it must fail to compile rather than strand a
// run at the first crash.
#[path = "../support.rs"]
mod support;

use mfm_capabilities::{EffectCapabilityContract, EntryAbsorbing, NoRefresh};
use support::{AutoincrementInsertRequest, Output};

enum AutoincrementInsertCapability {}

impl EffectCapabilityContract for AutoincrementInsertCapability {
    type Request = AutoincrementInsertRequest;
    type Returned = Output;
    type SafeFailure = Output;
    type Refresh = NoRefresh;
    type Entry = EntryAbsorbing<3>;
}

fn main() {}
