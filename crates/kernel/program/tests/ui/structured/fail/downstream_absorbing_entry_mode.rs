// The orphan attack. Orphan rules permit `impl ForeignTrait<LocalType> for
// ForeignType`, so a downstream crate must not be able to declare absorption
// over its own unkeyed request by implementing the evidence trait directly.
// This fails because the parameterized marker behind it is sealed.
#[path = "../support.rs"]
mod support;

use mfm_capabilities::{EffectEntryModeFor, EntryAbsorbing};
use support::AutoincrementInsertRequest;

impl EffectEntryModeFor<AutoincrementInsertRequest> for EntryAbsorbing<3> {}

fn main() {}
