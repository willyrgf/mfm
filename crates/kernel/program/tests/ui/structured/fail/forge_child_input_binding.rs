use mfm_ids::StableId;
use mfm_program::structured::ChildInputBinding;
use mfm_spec::structured::LexicalSlot;

fn forge(root_id: StableId, slot: LexicalSlot) -> ChildInputBinding {
    ChildInputBinding { root_id, slot }
}

fn main() {}
