use mfm_store::v1::CommittedRunJournal;

fn require_clone<T: Clone>() {}

fn main() {
    require_clone::<CommittedRunJournal>();
}
