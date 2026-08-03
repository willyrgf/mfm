use mfm_ids::ContentRef;
use mfm_program::structured::Capability;

struct CallerSelectedCapability;

impl Capability for CallerSelectedCapability {
    fn requirement_ref() -> mfm_program::Result<Option<ContentRef>> {
        Ok(None)
    }
}

fn main() {}
